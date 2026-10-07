//! Structured sanitizer response parsing and validation.
//!
//! Required shape:
//! ```json
//! { "text": "…", "changed": true, "warnings": [] }
//! ```

use serde::Deserialize;

/// Parsed sanitizer payload after strict validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredSanitizerResponse {
    pub text: String,
    pub changed: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawSanitizerJson {
    text: Option<serde_json::Value>,
    #[serde(default)]
    changed: Option<bool>,
    #[serde(default)]
    warnings: Option<Vec<String>>,
}

/// Strip optional markdown fences then parse/validate JSON.
pub fn parse_sanitizer_content(raw: &str) -> Result<StructuredSanitizerResponse, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("resposta vazia do sanitizer".into());
    }

    // Reject obvious non-JSON prose before attempting parse.
    if looks_like_forbidden_prose(trimmed) && !trimmed.starts_with('{') {
        return Err("resposta não é JSON (prosa/cabeçalho/glossário)".into());
    }

    let candidate = strip_code_fences(trimmed);
    let candidate = candidate.trim();

    // Must be a JSON object.
    if !candidate.starts_with('{') {
        return Err("resposta do sanitizer não começa com objeto JSON".into());
    }

    let parsed: RawSanitizerJson =
        serde_json::from_str(candidate).map_err(|e| format!("JSON inválido: {}", e))?;

    let text_val = parsed
        .text
        .ok_or_else(|| "JSON sem campo obrigatório \"text\"".to_string())?;

    let text = match text_val {
        serde_json::Value::String(s) => s,
        other => {
            return Err(format!(
                "campo \"text\" deve ser string, recebeu {}",
                other_type_name(&other)
            ));
        }
    };

    // Never accept JSON-as-text dump or nested structure pasted into text.
    let text_trim = text.trim().to_string();
    if text_trim.is_empty() {
        return Err("campo \"text\" está vazio".into());
    }
    if text_looks_like_json_dump(&text_trim) {
        return Err("campo \"text\" contém JSON cru ou estrutura inválida".into());
    }
    if looks_like_forbidden_prose(&text_trim) {
        return Err("campo \"text\" contém glossário/cabeçalho/explicação".into());
    }

    let changed = parsed.changed.unwrap_or(false);
    let warnings = parsed.warnings.unwrap_or_default();

    Ok(StructuredSanitizerResponse {
        text: text_trim,
        changed,
        warnings,
    })
}

fn strip_code_fences(s: &str) -> String {
    let t = s.trim();
    if !t.starts_with("```") {
        return t.to_string();
    }
    let mut lines: Vec<&str> = t.lines().collect();
    if lines
        .first()
        .is_some_and(|l| l.trim_start().starts_with("```"))
    {
        lines.remove(0);
    }
    if lines
        .last()
        .is_some_and(|l| l.trim() == "```" || l.trim().starts_with("```"))
    {
        lines.pop();
    }
    lines.join("\n")
}

fn looks_like_forbidden_prose(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    let markers = [
        "glossário",
        "glossario",
        "aqui está",
        "aqui esta",
        "here is",
        "here's",
        "texto final:",
        "transcrição:",
        "transcricao:",
        "explicação",
        "explicacao",
        "nota:",
        "### ",
        "## resumo",
        "como validador",
        "eu corrigi",
    ];
    markers.iter().any(|m| lower.contains(m))
}

fn text_looks_like_json_dump(s: &str) -> bool {
    let t = s.trim();
    if t.starts_with('{') && t.contains("\"text\"") {
        return true;
    }
    if t.starts_with("```") {
        return true;
    }
    false
}

fn other_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Result of content-type detection with scores (for tests / debug).
#[derive(Debug, Clone, PartialEq)]
pub struct ContentTypeScores {
    pub programming: f32,
    pub study: f32,
    pub general: f32,
    pub resolved: crate::pipeline_contract::ContentType,
}

/// Transparent multi-signal heuristic for Auto content type.
///
/// Combines:
/// - weighted lexicon hits (PT/EN + ASR spoken-code phrases)
/// - structural density (paths, extensions, camelCase, code punctuation)
/// - study/academic markers
/// - everyday-speech anchors that keep neutral content unbiased
///
/// Empty / tiny text → GeneralSpeech.
pub fn detect_content_type(text: &str) -> crate::pipeline_contract::ContentType {
    detect_content_type_scored(text).resolved
}

/// Same as [`detect_content_type`] but exposes scores.
pub fn detect_content_type_scored(text: &str) -> ContentTypeScores {
    use crate::pipeline_contract::ContentType;

    let raw = text.trim();
    if raw.chars().count() < 8 {
        return ContentTypeScores {
            programming: 0.0,
            study: 0.0,
            general: 1.0,
            resolved: ContentType::Auto,
        };
    }

    let lower = raw.to_lowercase();
    let words = tokenize_words(&lower);
    let word_count = words.len().max(1) as f32;

    // ── Lexicon: programming (weight, phrase) ─────────────────────────────
    // Higher weight = stronger exclusive signal.
    let prog_phrases: &[(&str, f32)] = &[
        // Keywords / syntax
        ("function ", 2.2),
        ("fn ", 2.4),
        ("const ", 1.8),
        ("let ", 1.4),
        ("var ", 1.2),
        ("import ", 2.0),
        ("export ", 1.8),
        ("from '", 1.6),
        ("from \"", 1.6),
        ("require(", 2.2),
        ("def ", 2.0),
        ("class ", 1.5),
        ("interface ", 2.0),
        ("type ", 1.0),
        ("enum ", 1.6),
        ("struct ", 2.0),
        ("impl ", 2.2),
        ("async ", 1.8),
        ("await ", 1.8),
        ("return ", 1.2),
        ("public ", 1.2),
        ("private ", 1.2),
        ("static ", 1.2),
        ("void ", 1.4),
        ("null", 0.8),
        ("undefined", 1.6),
        ("true", 0.3),
        ("false", 0.3),
        ("=>", 2.4),
        ("() =>", 2.6),
        ("::", 1.8),
        ("->", 1.6),
        ("```", 3.0),
        ("#!/", 2.5),
        // Tooling / stack
        ("npm ", 2.2),
        ("npx ", 2.0),
        ("yarn ", 1.8),
        ("pnpm ", 1.8),
        ("cargo ", 2.2),
        ("rustc ", 2.0),
        ("pip ", 1.8),
        ("pip3 ", 1.8),
        ("python ", 1.4),
        ("node ", 1.4),
        ("docker ", 1.8),
        ("kubectl ", 2.0),
        ("git ", 1.6),
        ("github", 1.2),
        ("gitlab", 1.2),
        ("commit ", 1.0),
        ("pull request", 1.8),
        ("merge request", 1.6),
        ("branch ", 1.0),
        ("repository", 1.2),
        ("repositório", 1.2),
        ("repositorio", 1.2),
        // Frameworks / libs (spoken + written)
        ("react", 1.6),
        ("useeffect", 2.4),
        ("usestate", 2.4),
        ("use memo", 1.8),
        ("usememo", 2.0),
        ("typescript", 2.0),
        ("javascript", 1.8),
        ("python", 1.2),
        ("rust", 1.2),
        ("tauri", 1.8),
        ("next.js", 2.0),
        ("nextjs", 2.0),
        ("vite", 1.4),
        ("webpack", 1.6),
        ("tokio", 1.8),
        ("serde", 1.8),
        ("django", 1.6),
        ("flask", 1.4),
        ("fastapi", 1.8),
        ("express", 1.2),
        ("graphql", 1.8),
        ("postgres", 1.6),
        ("mongodb", 1.6),
        ("redis", 1.4),
        ("kubernetes", 1.8),
        ("api ", 1.0),
        ("endpoint", 1.6),
        ("json", 1.4),
        ("yaml", 1.4),
        ("toml", 1.4),
        ("http", 1.0),
        ("https", 1.2),
        ("://", 2.2),
        ("localhost", 1.8),
        ("127.0.0.1", 2.0),
        // Paths / files
        (".rs", 2.0),
        (".ts", 1.8),
        (".tsx", 2.0),
        (".jsx", 1.8),
        (".js", 1.4),
        (".py", 1.8),
        (".go", 1.6),
        (".java", 1.6),
        (".kt", 1.6),
        (".cs", 1.4),
        (".cpp", 1.6),
        (".json", 1.6),
        (".yaml", 1.6),
        (".yml", 1.6),
        (".toml", 1.6),
        (".md", 1.0),
        (".env", 1.8),
        ("/src/", 2.2),
        ("\\src\\", 2.0),
        ("package.json", 2.4),
        ("cargo.toml", 2.4),
        ("node_modules", 2.2),
        ("dockerfile", 2.0),
        // CLI / ops spoken PT
        ("linha de comando", 1.8),
        ("terminal", 1.2),
        ("compilar", 1.4),
        ("build", 1.2),
        ("deploy", 1.4),
        ("debug", 1.4),
        ("debugger", 1.6),
        ("stack trace", 2.2),
        ("stacktrace", 2.2),
        ("exception", 1.6),
        ("erro de compilação", 2.0),
        ("erro de compilacao", 2.0),
        ("type error", 2.0),
        ("syntax error", 2.0),
        ("null pointer", 2.0),
        ("segmentation fault", 2.2),
        // ASR spoken punctuation / code dictation
        ("abre parênteses", 2.0),
        ("abre parenteses", 2.0),
        ("fecha parênteses", 2.0),
        ("fecha parenteses", 2.0),
        ("abre chaves", 2.0),
        ("fecha chaves", 2.0),
        ("abre colchetes", 1.8),
        ("fecha colchetes", 1.8),
        ("ponto e vírgula", 1.6),
        ("ponto e virgula", 1.6),
        ("arroba", 1.0),
        ("underline", 1.2),
        ("snake case", 2.0),
        ("camel case", 2.0),
        ("pascal case", 1.8),
        ("kebab case", 1.8),
        ("camelcase", 1.8),
        ("snake_case", 2.2),
        // Dev verbs common in dictation
        ("refator", 1.6),
        ("refactor", 1.6),
        ("implementa", 0.8),
        ("implementar", 0.8),
        ("commitar", 1.8),
        ("pushear", 1.8),
        ("pushar", 1.6),
        ("mergear", 1.8),
        ("clone o repo", 2.0),
        ("roda o teste", 1.6),
        ("rodar os testes", 1.8),
        ("unit test", 1.8),
        ("teste unitário", 1.8),
        ("teste unitario", 1.8),
        ("ci/cd", 2.0),
        ("pipeline", 1.0),
        ("pull request", 1.8),
        ("code review", 1.8),
        ("linter", 1.8),
        ("formatter", 1.4),
        ("prettier", 1.6),
        ("eslint", 1.8),
        ("clippy", 1.8),
    ];

    // ── Lexicon: study / academic ─────────────────────────────────────────
    let study_phrases: &[(&str, f32)] = &[
        ("definição", 2.2),
        ("definicao", 2.2),
        ("conceito", 1.8),
        ("teorema", 2.6),
        ("lema ", 2.0),
        ("corolário", 2.4),
        ("corolario", 2.4),
        ("proposição", 2.0),
        ("proposicao", 2.0),
        ("hipótese", 1.8),
        ("hipotese", 1.8),
        ("tese", 1.4),
        ("equação", 2.2),
        ("equacao", 2.2),
        ("fórmula", 1.8),
        ("formula", 1.4),
        ("demonstração", 2.2),
        ("demonstracao", 2.2),
        ("prova que", 1.8),
        ("demonstrar que", 2.0),
        ("capítulo", 1.8),
        ("capitulo", 1.8),
        ("seção", 1.4),
        ("secao", 1.4),
        ("exercício", 2.0),
        ("exercicio", 2.0),
        ("questão", 1.2),
        ("questao", 1.2),
        ("em outras palavras", 1.8),
        ("por exemplo", 1.2),
        ("isto é", 1.0),
        ("ou seja", 1.0),
        ("segundo a teoria", 2.2),
        ("de acordo com", 1.2),
        ("conforme o autor", 2.0),
        ("bibliografia", 2.2),
        ("referência bibliográfica", 2.4),
        ("referencia bibliografica", 2.4),
        ("resumo", 0.8),
        ("introdução", 1.0),
        ("introducao", 1.0),
        ("conclusão", 1.0),
        ("conclusao", 1.0),
        ("metodologia", 2.0),
        ("método científico", 2.2),
        ("metodo cientifico", 2.2),
        ("experimento", 1.6),
        ("amostra", 1.2),
        ("variável", 1.4),
        ("variavel", 1.4),
        ("hipótese nula", 2.4),
        ("hipotese nula", 2.4),
        ("significância estatística", 2.4),
        ("significancia estatistica", 2.4),
        ("p-valor", 2.2),
        ("desvio padrão", 2.0),
        ("desvio padrao", 2.0),
        ("média", 0.6),
        ("mediana", 1.4),
        ("histograma", 1.8),
        ("derivada", 2.0),
        ("integral", 1.8),
        ("limite de", 1.2),
        ("função contínua", 2.0),
        ("funcao continua", 2.0),
        ("matriz", 1.4),
        ("vetor", 1.2),
        ("probabilidade", 1.8),
        ("distribuição normal", 2.2),
        ("distribuicao normal", 2.2),
        ("fotossíntese", 2.2),
        ("fotossintese", 2.2),
        ("mitocôndria", 2.2),
        ("mitocondria", 2.2),
        ("dna", 1.4),
        ("rna", 1.4),
        ("átomo", 1.6),
        ("atomo", 1.6),
        ("molécula", 1.6),
        ("molecula", 1.6),
        ("lei de newton", 2.4),
        ("primeira lei", 1.2),
        ("segunda lei", 1.2),
        ("terceira lei", 1.2),
        ("revolução francesa", 2.0),
        ("revolucao francesa", 2.0),
        ("idade média", 1.8),
        ("idade media", 1.8),
        ("século", 1.2),
        ("seculo", 1.2),
        ("literatura", 1.4),
        ("poesia", 1.4),
        ("narrador", 1.6),
        ("protagonista", 1.6),
        ("metáfora", 1.8),
        ("metafora", 1.8),
        ("figura de linguagem", 2.2),
        ("análise textual", 2.0),
        ("analise textual", 2.0),
        ("resuma o texto", 1.8),
        ("explique o conceito", 2.2),
        ("o que significa", 1.0),
        ("qual a diferença entre", 1.6),
        ("quais são as características", 1.8),
        ("quais sao as caracteristicas", 1.8),
        ("aprenda", 0.6),
        ("estudar", 1.0),
        ("estudo sobre", 1.6),
        ("aula de", 1.4),
        ("professor", 1.0),
        ("aluno", 0.8),
        ("prova amanhã", 1.8),
        ("prova amanha", 1.8),
        ("trabalho de faculdade", 2.0),
        ("tcc", 1.6),
        ("dissertação", 2.0),
        ("dissertacao", 2.0),
        ("monografia", 2.0),
        ("artigo científico", 2.4),
        ("artigo cientifico", 2.4),
        ("peer review", 2.0),
        ("citações", 1.6),
        ("citacoes", 1.6),
        ("abnt", 2.0),
        ("apa ", 1.4),
    ];

    // ── Lexicon: general speech (damps code/study when dominant) ───────────
    let general_phrases: &[(&str, f32)] = &[
        ("oi ", 1.2),
        ("olá", 1.0),
        ("ola ", 1.0),
        ("tudo bem", 1.6),
        ("bom dia", 1.4),
        ("boa tarde", 1.4),
        ("boa noite", 1.4),
        ("obrigado", 1.2),
        ("obrigada", 1.2),
        ("por favor", 1.0),
        ("me lembra", 1.4),
        ("não esquece", 1.4),
        ("nao esquece", 1.4),
        ("compra ", 1.0),
        ("mercado", 1.2),
        ("almoço", 1.2),
        ("almoco", 1.2),
        ("jantar", 1.2),
        ("reunião", 1.0),
        ("reuniao", 1.0),
        ("amanhã", 0.8),
        ("amanha", 0.8),
        ("hoje ", 0.6),
        ("ontem", 0.8),
        ("fim de semana", 1.4),
        ("família", 1.2),
        ("familia", 1.2),
        ("filho", 0.8),
        ("filha", 0.8),
        ("esposa", 1.0),
        ("marido", 1.0),
        ("namorado", 1.2),
        ("namorada", 1.2),
        ("filme", 1.0),
        ("série", 1.0),
        ("serie", 1.0),
        ("netflix", 1.4),
        ("futebol", 1.4),
        ("jogo ", 0.6),
        ("viagem", 1.2),
        ("passagem", 1.0),
        ("uber", 1.2),
        ("ifood", 1.4),
        ("whatsapp", 1.2),
        ("mensagem", 0.8),
        ("ligação", 1.0),
        ("ligacao", 1.0),
        ("te ligo", 1.4),
        ("me liga", 1.4),
        ("beijo", 1.4),
        ("abraço", 1.2),
        ("abraco", 1.2),
        ("saudade", 1.4),
        ("parabéns", 1.2),
        ("parabens", 1.2),
        ("aniversário", 1.4),
        ("aniversario", 1.4),
        ("clima", 0.8),
        ("chuva", 1.0),
        ("trânsito", 1.2),
        ("transito", 1.2),
        ("médico", 1.0),
        ("medico", 1.0),
        ("remédio", 1.2),
        ("remedio", 1.2),
        ("consulta", 1.0),
    ];

    let mut prog = score_phrases(&lower, prog_phrases);
    let mut study = score_phrases(&lower, study_phrases);
    let mut general = score_phrases(&lower, general_phrases);

    // ── Structural signals (regex-free, char scans) ───────────────────────
    prog += structural_programming_score(raw, &lower, &words);

    // Length normalization: short dictation shouldn't need many hits
    let len_factor = (word_count / 12.0).clamp(0.65, 1.35);
    prog /= len_factor;
    study /= len_factor;
    general /= len_factor;

    // Base prior for everyday speech (slight)
    general += 0.35;

    // If programming structural density is high, boost further
    let punct_density = code_punct_density(raw);
    if punct_density >= 0.04 {
        prog += 2.0;
    } else if punct_density >= 0.02 {
        prog += 1.0;
    }

    // Cross-damping: strong general chat reduces weak code single-hits like "http" in URLs shared casually
    if general > prog && general > study {
        prog *= 0.75;
        study *= 0.85;
    }
    // Strong code damps study false friends ("classe", "função" alone)
    if prog >= 3.5 {
        study *= 0.7;
    }
    // Strong study damps weak code
    if study >= 3.5 && prog < 2.5 {
        prog *= 0.75;
    }

    // Decision with margins
    let resolved = decide_content_type(prog, study, general);

    ContentTypeScores {
        programming: prog,
        study,
        general,
        resolved,
    }
}

fn score_phrases(lower: &str, phrases: &[(&str, f32)]) -> f32 {
    let mut score = 0.0f32;
    let mut matched = 0u32;
    for (phrase, w) in phrases {
        if lower.contains(phrase) {
            score += *w;
            matched += 1;
            // Diminishing returns after many hits of same class
            if matched > 8 {
                score += w * 0.35;
            }
        }
    }
    score
}

fn tokenize_words(lower: &str) -> Vec<&str> {
    lower
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|w| !w.is_empty())
        .collect()
}

fn structural_programming_score(raw: &str, lower: &str, words: &[&str]) -> f32 {
    let mut s = 0.0f32;

    // URL / path-like
    if lower.contains("http://") || lower.contains("https://") {
        s += 1.8;
    }
    if lower.contains("www.") {
        s += 1.0;
    }
    // Windows or unix path fragments
    if raw.contains(":\\") || raw.contains(":/") {
        s += 2.0;
    }
    if lower.matches('/').count() >= 2
        && (lower.contains("src") || lower.contains("app") || lower.contains("lib"))
    {
        s += 1.8;
    }
    // file.ext patterns
    let ext_hits = [
        ".rs", ".ts", ".tsx", ".jsx", ".js", ".py", ".go", ".java", ".kt", ".cs", ".cpp", ".h",
        ".hpp", ".json", ".yaml", ".yml", ".toml", ".sql", ".sh", ".ps1", ".wasm",
    ]
    .iter()
    .filter(|e| lower.contains(**e))
    .count();
    s += (ext_hits as f32) * 1.5;

    // camelCase / PascalCase / snake_case token ratios
    let mut camel = 0u32;
    let mut snake = 0u32;
    let mut screamy = 0u32;
    for w in words {
        if w.len() >= 4
            && w.contains('_')
            && w.chars()
                .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
        {
            snake += 1;
        }
        if looks_camel_case(w) {
            camel += 1;
        }
        if w.len() >= 3
            && w.chars()
                .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
            && w.contains('_')
        {
            screamy += 1;
        }
    }
    if camel >= 2 {
        s += 1.6 + (camel as f32 - 2.0) * 0.35;
    } else if camel == 1 {
        s += 0.7;
    }
    if snake >= 2 {
        s += 1.8;
    } else if snake == 1 {
        s += 0.9;
    }
    if screamy >= 1 {
        s += 1.2;
    }

    // Brackets / operators density helpers already elsewhere; count pairs
    let pairs = count_balancedish(raw);
    s += pairs;

    // Version numbers x.y.z often technical
    if has_semver_like(lower) {
        s += 1.2;
    }

    // Hex colors / 0x
    if lower.contains("0x") || lower.contains("#fff") || lower.contains("#000") {
        s += 1.4;
    }

    s
}

fn looks_camel_case(w: &str) -> bool {
    if w.len() < 4 || !w.is_ascii() {
        return false;
    }
    let bytes = w.as_bytes();
    let has_lower = bytes.iter().any(|b| b.is_ascii_lowercase());
    let has_upper = bytes.iter().any(|b| b.is_ascii_uppercase());
    if !has_lower || !has_upper {
        return false;
    }
    // internal uppercase (camel/Pascal)
    bytes[1..].iter().any(|b| b.is_ascii_uppercase())
}

fn code_punct_density(raw: &str) -> f32 {
    if raw.is_empty() {
        return 0.0;
    }
    let special = raw
        .chars()
        .filter(|c| {
            matches!(
                c,
                '{' | '}'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | ';'
                    | '='
                    | '<'
                    | '>'
                    | '|'
                    | '&'
                    | '*'
                    | '`'
                    | '\\'
            )
        })
        .count();
    special as f32 / raw.chars().count() as f32
}

fn count_balancedish(raw: &str) -> f32 {
    let mut s = 0.0f32;
    let open_b = raw.chars().filter(|c| *c == '{').count();
    let close_b = raw.chars().filter(|c| *c == '}').count();
    let open_p = raw.chars().filter(|c| *c == '(').count();
    let close_p = raw.chars().filter(|c| *c == ')').count();
    if open_b > 0 && close_b > 0 {
        s += 1.8;
    }
    if open_p >= 2 && close_p >= 2 {
        s += 1.2;
    }
    if raw.contains("[]") || raw.contains("()") {
        s += 1.0;
    }
    s
}

fn has_semver_like(lower: &str) -> bool {
    // simple scan for n.n or n.n.n
    let b = lower.as_bytes();
    let mut i = 0;
    while i + 2 < b.len() {
        if b[i].is_ascii_digit() {
            let mut j = i;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j < b.len() && b[j] == b'.' {
                let mut k = j + 1;
                if k < b.len() && b[k].is_ascii_digit() {
                    while k < b.len() && b[k].is_ascii_digit() {
                        k += 1;
                    }
                    // optional third component
                    if k + 1 < b.len() && b[k] == b'.' && b[k + 1].is_ascii_digit() {
                        return true;
                    }
                    // two-part version near code words is weak; still count
                    if j - i <= 3 && k - (j + 1) <= 3 {
                        return true;
                    }
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    false
}

fn decide_content_type(
    prog: f32,
    study: f32,
    general: f32,
) -> crate::pipeline_contract::ContentType {
    use crate::pipeline_contract::ContentType;

    // Clear programming
    if prog >= 2.2 && prog >= study + 0.6 && prog >= general + 0.35 {
        return ContentType::Programming;
    }
    // Clear study
    if study >= 2.4 && study >= prog + 0.7 && study >= general + 0.35 {
        return ContentType::Study;
    }
    // Soft programming (one strong structural/tool hit)
    if prog >= 1.6 && prog > study && prog >= general {
        return ContentType::Programming;
    }
    // Soft study
    if study >= 1.8 && study > prog && study >= general {
        return ContentType::Study;
    }
    // Tie-breakers
    let max = prog.max(study).max(general);
    if (prog - max).abs() < f32::EPSILON && prog >= 1.4 && prog >= study {
        return ContentType::Programming;
    }
    if (study - max).abs() < f32::EPSILON && study >= 1.6 {
        return ContentType::Study;
    }
    ContentType::Auto
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline_contract::ContentType;

    #[test]
    fn accepts_valid_json() {
        let raw = r#"{"text":"Olá mundo","changed":true,"warnings":["x"]}"#;
        let p = parse_sanitizer_content(raw).unwrap();
        assert_eq!(p.text, "Olá mundo");
        assert!(p.changed);
        assert_eq!(p.warnings, vec!["x"]);
    }

    #[test]
    fn accepts_fenced_json() {
        let raw = "```json\n{\"text\":\"ok\",\"changed\":false,\"warnings\":[]}\n```";
        let p = parse_sanitizer_content(raw).unwrap();
        assert_eq!(p.text, "ok");
        assert!(!p.changed);
    }

    #[test]
    fn rejects_missing_text() {
        let raw = r#"{"changed":true,"warnings":[]}"#;
        assert!(parse_sanitizer_content(raw).is_err());
    }

    #[test]
    fn rejects_prose() {
        let raw = "Aqui está a transcrição corrigida: olá";
        assert!(parse_sanitizer_content(raw).is_err());
    }

    #[test]
    fn rejects_glossary_in_text_field() {
        let raw = r#"{"text":"Glossário:\n- foo","changed":false,"warnings":[]}"#;
        assert!(parse_sanitizer_content(raw).is_err());
    }

    #[test]
    fn rejects_json_dump_in_text() {
        let raw = r#"{"text":"{\"text\":\"nested\"}","changed":false,"warnings":[]}"#;
        assert!(parse_sanitizer_content(raw).is_err());
    }

    #[test]
    fn rejects_empty_text() {
        let raw = r#"{"text":"   ","changed":false,"warnings":[]}"#;
        assert!(parse_sanitizer_content(raw).is_err());
    }

    #[test]
    fn detect_programming_tools() {
        let t = "rode npm install e depois cargo build no /src/app";
        assert_eq!(detect_content_type(t), ContentType::Programming);
    }

    #[test]
    fn detect_programming_spoken_asr() {
        let t = "cria uma function async que faz await no fetch e usa useEffect no React";
        assert_eq!(detect_content_type(t), ContentType::Programming);
    }

    #[test]
    fn detect_programming_camel_and_path() {
        let t = "abre o arquivo src/components/UserProfile.tsx e corrige getUserById";
        assert_eq!(detect_content_type(t), ContentType::Programming);
    }

    #[test]
    fn detect_programming_brackets() {
        let t = "const x = () => { return foo.bar; };";
        assert_eq!(detect_content_type(t), ContentType::Programming);
    }

    #[test]
    fn detect_study() {
        let t = "A definição do conceito é clara. Por exemplo, o teorema diz que...";
        assert_eq!(detect_content_type(t), ContentType::Study);
    }

    #[test]
    fn detect_study_academic() {
        let t =
            "Na metodologia do artigo científico a hipótese nula foi rejeitada com p-valor baixo";
        assert_eq!(detect_content_type(t), ContentType::Study);
    }

    #[test]
    fn detect_general_chat() {
        let t = "Oi, tudo bem? Bom dia, não esquece de comprar pão no mercado depois do almoço";
        assert_eq!(detect_content_type(t), ContentType::Auto);
    }

    #[test]
    fn detect_general_not_fooled_by_http_mention_alone_in_chat() {
        // Everyday message should stay general even if a weak tech word appears in isolation
        // when general signals dominate.
        let t = "Oi amor, te ligo depois do jantar, beijo e boa noite, saudades";
        assert_eq!(detect_content_type(t), ContentType::Auto);
    }

    #[test]
    fn detect_empty_is_general() {
        assert_eq!(detect_content_type("oi"), ContentType::Auto);
        assert_eq!(detect_content_type(""), ContentType::Auto);
    }

    #[test]
    fn detect_scores_expose_winner() {
        let s = detect_content_type_scored(
            "git commit e push no github depois do code review do pull request",
        );
        assert_eq!(s.resolved, ContentType::Programming);
        assert!(s.programming > s.study);
        assert!(s.programming > s.general);
    }
}
