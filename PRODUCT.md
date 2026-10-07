# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Pessoas que usam Windows e precisam ditar texto com frequência em qualquer aplicativo, mantendo o foco no trabalho em vez de administrar uma ferramenta de transcrição.

## Product Purpose

Sonora transforma fala em texto por meio de um atalho global, cola o resultado no campo atualmente focado e mantém um hub local para gerenciar pipelines, histórico, arquivos, vocabulário, atalhos e preferências. O sucesso é um fluxo de ditado rápido, confiável e recuperável que exige pouca atenção visual durante o uso cotidiano.

## Positioning

O aplicativo combina uma interface desktop local de gerenciamento com um gadget always-on-top usado como interface primária de ditado em qualquer outro aplicativo, preservando controle técnico sobre pipelines, providers, modelos, fallback e vocabulário.

## Operating Context

- O Sonora aberto funciona como hub de gerenciamento.
- O gadget e os atalhos globais funcionam como interface cotidiana de ditado.
- `Ctrl+B` inicia ou encerra a gravação por padrão; `Ctrl+Q` cancela.
- A transcrição de microfone cola o texto no campo focado; upload manual de arquivo não cola automaticamente.
- Histórico, configurações, chaves de API e áudios ficam armazenados localmente no Windows.

## Capabilities and Constraints

- Aplicativo desktop Tauri 2 com frontend React 18/TypeScript e backend Rust.
- Captura de microfone, nível de áudio real, upload de arquivos, histórico, edição, cópia, retranscrição, retry, avaliação de pronúncia e revelação de áudio no Explorer.
- Pipelines de produto com OpenRouter/Groq Whisper e Google Gemini, além de caminhos legados e experimentais preservados sob divulgação progressiva.
- Providers e múltiplas chaves de API permanecem configuráveis e mascarados na interface; nenhum segredo pode aparecer em logs, screenshots ou documentação.
- Vocabulário estruturado preserva grafia, variações faladas, categoria, literalidade e estado ativo.
- Gadget deve preservar hotkeys globais, always-on-top, foco, posicionamento e recuperação de falha disponíveis na arquitetura real.
- Não há cloud sync, Help & docs, What's new ou outras capacidades meramente sugeridas pelos mockups.
- Idioma e copy seguem a estratégia atual do produto, predominantemente pt-BR.

## Brand Commitments

- Nome do produto: Sonora; a marca compacta pode usar “Sonora”.
- Identidade própria, sem copiar marca, logo ou assets do Wispr Flow.
- Interface quase monocromática, silenciosa, premium e desktop-native, com tema claro e escuro que segue o Windows por padrão; sem laranja.
- Princípio visual e comportamental vinculante: “Quiet by default. Information appears when needed.”

## Evidence on Hand

- Implementação atual em `src/` e `src-tauri/`, autoridade para funções, estados, conteúdo e restrições.
- Documentação técnica em `README.md` e `docs/`.
- Nove mockups fornecidos pelo usuário como direção estética e composicional, sem autoridade para inventar capacidades.
- Wispr Flow é apenas referência conceitual de hierarquia, silêncio visual e interação do gadget.

## Product Principles

1. Simplificar a superfície sem simplificar a capacidade do produto.
2. Manter o ditado disponível em qualquer aplicativo sem exigir a janela principal.
3. Exibir informação e controles somente quando o contexto ou o estado exigir.
4. Preservar recuperação, diagnóstico e controle técnico por divulgação progressiva.
5. Tratar dados, áudios e credenciais como estado local sensível do usuário.

## Accessibility & Inclusion

A interface deve preservar contraste WCAG, navegação por teclado, foco visível, nomes acessíveis, labels e associações de erro, estados disabled/loading, hit targets adequados e suporte a movimento reduzido.
