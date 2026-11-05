# AssemblyAI Universal-3.6 Pro + contexto inteligente

## Configuração

1. Em **Ajustes → Transcrição**, escolha **Rápido e preciso**, **Preciso** ou **Ultrapreciso**.
2. Selecione **OpenRouter**, **ID customizado** e `assemblyai/universal-3-6-pro`.
3. O Sonora passa a encaminhar a grafia canônica dos termos habilitados do vocabulário ao `keyterms_prompt` (até 100 termos). A regra de literais continua no pós-processamento.
4. Em **Contexto inteligente**, ative o recurso se desejar compartilhar uma imagem da janela em foco e até cinco ou dez transcrições anteriores com um modelo visual. O padrão é `nex-agi/nex-n2.5-mini`, mas aceita outro ID multimodal da OpenRouter.

O idioma **não** é forçado para português nos requests da AssemblyAI. O prompt solicita reconhecimento automático e preservação de code-switching entre português e inglês. O comportamento depende do modelo em uso; teste com áudios representativos.

## Fluxo e privacidade

- Por padrão, o **contexto inteligente fica desligado**. A extração de imagem é feita **apenas para ditados de microfone**, nunca para importações de arquivo, e apenas quando uma rota AssemblyAI está selecionada.
- A captura usa a **janela ativa no início da gravação**. Caso o foco tenha mudado, a captura é descartada.
- O bitmap é reduzido a no máximo 1280 × 900, codificado como PNG em memória e enviado ao modelo multimodal pela OpenRouter. **Imagem, resumo e trechos de histórico não são armazenados em disco** por essa funcionalidade.
- As últimas cinco/dez transcrições de microfone são encaminhadas como contexto conversacional, em ordem cronológica. A visão tenta identificar assunto e termos; não reescreve o áudio e deve retornar JSON estruturado.
- A análise visual começa junto com a gravação. Ao parar, espera no máximo 200 ms. Se houver timeout ou erro, a transcrição segue com seu vocabulário e, quando ativado, as últimas transcrições, sem bloquear o usuário.
- A detecção de idioma ocorre no modelo ASR; o modelo visual **não** recebe o áudio. O áudio vai somente para o endpoint de transcrição.
- O Sonora cancela e descarta o contexto temporário no encerramento ou cancelamento do ditado. O histórico existente continua submetido às preferências de armazenamento já vigentes.
- O usuário deve evitar habilitar a captura em janelas com dados confidenciais. A implementação não promete ocultação automática de informações sensíveis presentes na imagem.

## Integração

O cliente `src-tauri/src/openrouter.rs` envia áudio ao endpoint `/api/v1/audio/transcriptions`. Para IDs `assemblyai/universal-3*`, usa:

```json
{
  "provider": {
    "options": {
      "assemblyai": {
        "prompt": "Transcribe faithfully ...",
        "keyterms_prompt": ["Sonora", "Rust"],
        "conversation_context": ["Transcrição anterior"]
      }
    }
  }
}
```

Essas opções são serializadas no campo multipart `provider`. O campo `language` fica **ausente** na rota AssemblyAI para permitir detecção automática. Outras rotas preservam seu comportamento.

## Validação

O workflow **Windows qualification** deve passar em: `npm run lint`, `npm run test:frontend`, `npm run test:ui`, `npm run build`, `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets -- -D warnings` e `cargo test --locked`.

Antes de colocar em produção, usar uma chave OpenRouter de teste e um conjunto próprio de áudios em português brasileiro e português/inglês misturados. Comparar versão sem contexto e com contexto, erros de termos, falsos positivos, tempo total e custo. Também conferir captura de tela no Windows com monitores múltiplos, DPI alto e programas protegidos. O CI não executa requisições pagas reais.

### Referências

- [AssemblyAI: Sync Speech-to-Text via OpenRouter](https://www.assemblyai.com/blog/aai-on-openrouter)
- [OpenRouter: Universal-3.6 Pro](https://openrouter.ai/assemblyai/universal-3-6-pro)
- [Microsoft: GetDIBits](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-getdibits)
