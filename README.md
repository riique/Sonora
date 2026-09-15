# Sonora

Aplicativo desktop de **digitação por voz** para Windows. Grave com um atalho global, o app transcreve com motores em nuvem, opcionalmente refina o texto e cola no campo focado (`Ctrl+V`).

**Versão:** 2.0.3 · **Stack:** Tauri 2 · React 18 · TypeScript · Rust

## Sonora v2.0

- **Sua voz, mais simples:** um retrato curto, expressões e hábitos de fala, com medições e configurações disponíveis nos detalhes.
- **Atualização visível:** o botão do retrato permanece à vista com a quantidade de palavras que falta; ele é habilitado quando você pode gerar uma nova versão.
- **Silêncio tratado no computador:** uma gravação claramente sem voz mostra “Nenhuma voz encontrada” na barra e encerra sem enviar áudio ou colar texto. Falas curtas e baixas são preservadas por uma verificação conservadora.
- **Entrega flexível no campo em foco:** comece a falar em um aplicativo e mude para outro (como uma janela de chat ou editor); ao terminar a transcrição, o texto vai diretamente para o local selecionado.
- **Nova identidade:** Sonora na interface, no executável e nos instaladores. Seus dados e perfis existentes continuam disponíveis.

[Mudanças, compatibilidade e atualização da instalação anterior](docs/SONORA_2.0.md). [Código no GitHub](https://github.com/riique/Sonora).

---

## O que faz

- Gravação pelo microfone (atalho global ou botão na UI)
- Transcrição em nuvem com **pipelines de produto** (OpenRouter/Groq Whisper + Google Gemini)
- Cola automática no campo focado (clipboard + simulação de paste)
- Upload de arquivos de áudio (WAV, MP3, etc.)
- Histórico local sem limite artificial, com áudio revelável no Explorer, retranscrição e avaliação de pronúncia
- Modelo customizado por pipeline via Google AI Studio ou OpenRouter (LLM multimodal ou STT dedicado no OpenRouter)
- FileTagging opcional para converter referências faladas em menções como `@index.tsx` nos modelos multimodais
- Vocabulário estruturado (termos, aliases, categorias, literais strict)
- Overlay **gadget** sempre no topo + recuperação direta quando uma transcrição falha
- Atalhos globais configuráveis (padrão: `Ctrl+B` grava, `Ctrl+Q` cancela)

---

## Correções da auditoria — 1.0.34

- Chaves protegidas por DPAPI da conta Windows; somente referências opacas chegam à interface.
- Escritas atômicas, histórico incremental paginado e recuperação de itens removidos.
- Captura incremental com limite de 15 minutos, recuperação de áudio interrompido e cancelamento do processamento de ditados.
- Coleta de contexto do navegador somente por solicitação vigente, conforme as fontes habilitadas.
- Verificação do campo de destino antes do paste; resultado e falha de entrega permanecem disponíveis no Histórico.
- Diagnóstico local, backup com áudio opcional, arquivamento e seleção rápida de destino/Style.
- Dependências corrigidas, permissões separadas por janela e CI Windows com verificações de contratos.

Qualificação e limites: [auditoria implementada](docs/audit-remediation.md). Procedimentos de dados e distribuição: [recuperação e release](docs/recovery-and-release.md).

## Novidades — pipelines de transcrição

A orquestração saiu de um monólito em `audio.rs` para módulos dedicados (`transcription/`, `gemini/`, contratos e vocabulário). A UI de **Configurações** é centrada nos pipelines de produto ativos.

### Modos de produto

| Modo | Fluxo | Sanitizer | Fallback típico |
|------|--------|-----------|-----------------|
| ⚡ **Ultrarrápido** | OpenRouter STT → Whisper (Groq fixo) | Não | — |
| 🚀 **Rápido e preciso** | Gemini (Files API / inline) | Não | Whisper (configurável) |
| 🎯 **Preciso** | Whisper ∥ upload → refine Gemini | Não | Whisper ou Gemini puro |
| 💎 **Ultrapreciso** | Whisper ∥ upload → sanitizer → Gemini | Sim (JSON) | Texto sanitizado / Whisper |

### Prompt universal e FileTagging

Os modelos Gemini multimodais recebem uma `systemInstruction` única que trata, por trecho, conversa comum, programação e conteúdo acadêmico/científico. Não é necessário escolher previamente um tipo de conteúdo.

O botão **FileTagging** em Configurações ativa ou desativa a regra que converte referências inequívocas a arquivos em texto simples como `@src/components/Header.tsx`. A função prepara a menção no texto; a integração do IDE/chat continua sob responsabilidade do aplicativo de destino.

### Outras melhorias desta entrega

- **Módulo Gemini** — Files API, STT, refine (preciso/ultrapreciso), transporte e avaliação de pronúncia
- **Sanitizer JSON** — saída estruturada no modo Ultrapreciso, com fallback para texto bruto se o formato falhar
- **Vocabulário estruturado** — canônico + aliases + categoria + flag *strict* (substitui a lista simples de palavras)
- **Histórico observável** — modo, modelos, estágios, textos intermediários, fallback, latências; copiar / editar / excluir / detalhes / retranscrever / pronúncia
- **Áudio configurável** — escolha a pasta para novas gravações transcritas e abra cada arquivo diretamente pelo Histórico
- **Normalização sensível a ruído** — ganho adaptativo limitado, pausas sem amplificação de room tone, limiter em -3 dBFS e original preservado como `.original.wav`
- **UI de Configurações** — cards de pipeline e botão persistente de FileTagging
- **Roteamento customizado** — presets ou ID livre por pipeline; OpenRouter separa Chat Completions multimodal de Speech-to-Text dedicado
- **Whisper no Ultrarrápido** — escolha entre `openai/whisper-large-v3-turbo` e `openai/whisper-large-v3`, sempre pelo provedor Groq no OpenRouter
- **Recuperação no gadget** — falhas exibem uma ação **Regenerar** usando o áudio já salvo, sem abrir o Histórico
- **Telemetria local** — latência por estágio, RTF estimado, throughput (sem analytics externo)
- Testes unitários Rust no pipeline (`cargo test`)

### Arquitetura (visão rápida)

```text
src-tauri/src/
├── audio.rs                 # captura mic, WAV, clipboard/paste
├── transcription/           # legado + modos de produto + telemetria
├── gemini/                  # Files API, STT, refine, pronúncia
├── pipeline_contract.rs     # TranscriptionMode, configuração e estágios
├── vocabulary.rs            # termos estruturados
├── sanitizer_json.rs        # parse da saída JSON do sanitizer
├── groq.rs                  # Whisper e sanitizer
└── history.rs / settings.rs

src/views/
├── ConfiguracoesView.tsx    # pipelines + FileTagging + vocabulário
├── HistoricoView.tsx        # cards, detalhes, ações
└── TranscricaoView.tsx      # upload de arquivo
```

Documentação técnica da migração: [`docs/TRANSCRIPTION_MIGRATION_FINAL_REPORT.md`](docs/TRANSCRIPTION_MIGRATION_FINAL_REPORT.md).

---

## Requisitos

1. **Node.js 24** (npm)
2. **Rust 1.97.1** via [rustup](https://rustup.rs/)
3. Chaves de API (conforme o modo):
   - **Groq** — Preciso, Ultrapreciso, sanitizer e fallbacks legados
   - **Google (Gemini)** — Rápido e preciso, Preciso, Ultrapreciso, pronúncia
   - **OpenRouter** — obrigatório no Ultrarrápido; também aceita modelos multimodais com áudio e modelos dedicados de transcrição

---

## Desenvolvimento

```bash
npm install
npm run tauri dev
```

Só a UI (sem backend nativo completo):

```bash
npm run dev
```

---

## Build de produção

```bash
npm run tauri build
```

Se o `cargo` não estiver no `PATH` (Windows / PowerShell):

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;" + $env:PATH
npm run tauri build
```

**Artefatos típicos:**

- Executável: `src-tauri/target/release/sonora.exe`
- Instaladores: `src-tauri/target/release/bundle/nsis/` e `bundle/msi/`

Detalhes: [`BUILD.md`](BUILD.md).

---

## Uso rápido

1. Abra **Configurações** e salve as chaves de API necessárias.
2. Escolha um **pipeline** (Ultrarrápido, Rápido e preciso, Preciso ou Ultrapreciso).
3. Foque o campo de texto de destino e use o atalho de gravação (`Ctrl+B` por padrão).
4. Pare a gravação com o mesmo atalho; o texto final é colado no campo focado e entra no **Histórico**.
5. Cancele com `Ctrl+Q` (sem gerar texto novo).

**Notas:**

- Gravação pelo mic **cola** no campo focado; upload de arquivo **não** cola automaticamente.
- Arquivos de áudio grandes (> ~50 MB) são rejeitados.
- Dados locais (histórico, settings, chaves, áudios): `%APPDATA%\com.haumeavoice.app\`

O identificador interno de dados mantém o nome legado para preservar sua instalação. Para atualizar a instalação anterior para a pasta Sonora com backup e verificação de dados, siga [o procedimento Windows](docs/SONORA_2.0.md#instalação-windows).

---

## Telas

| View | Função |
|------|--------|
| Início | Status, contadores, iniciar gravação |
| Transcrição | Upload de arquivo |
| Histórico | Entradas, métricas, retranscrever, pronúncia |
| Atalhos | Rebind de toggle/cancel |
| Configurações | Pipelines, chaves, vocabulário, avançado |
| Gadget | Overlay compacto always-on-top |

---

## Stack

| Camada | Tecnologia |
|--------|------------|
| Shell | Tauri 2 |
| Frontend | React 18, TypeScript, Vite 5, Tailwind CSS 3 |
| Backend | Rust (captura, STT, IPC, OS) |
| Áudio | cpal (WASAPI no Windows) |
| STT / LLM | Groq Whisper, Gemini, Deepgram (legado) |
| Clipboard / paste | arboard + enigo |

---

## Testes

```bash
cd src-tauri
cargo test
```

Checklist manual: [`docs/MANUAL_TEST_CHECKLIST.md`](docs/MANUAL_TEST_CHECKLIST.md).

---

## Licença

Consulte o repositório para a licença aplicável. Chaves de API e dados de uso são de responsabilidade do usuário; as chaves são protegidas pelo DPAPI da conta Windows; a interface recebe apenas referências opacas.
