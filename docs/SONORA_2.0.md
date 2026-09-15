# Sonora v2.0

Sonora é o novo nome da geração Tauri do aplicativo de ditado. A versão 2.0 foi lançada como `2.0.0`; a versão atual é `2.0.3`.

### Atualização 2.0.3

Atualização do modelo padrão de geração de retrato por IA (Voice Profile) via OpenRouter para `meta/muse-spark-1.3-contributor`, sucedendo a versão 1.2.

### Correção 2.0.2

Permite iniciar uma transcrição em um aplicativo e alternar para outro (por exemplo, clicar em uma caixa de chat ou editor) antes ou durante o processamento. Ao concluir a transcrição, o texto é colado diretamente no local selecionado e com foco ativo naquele momento, sem interrupções por "campo de destino mudou". O texto permanece também copiado na área de transferência.

### Correção 2.0.1

O botão de criar ou atualizar o retrato permanece visível quando o retrato com IA está ativado. Enquanto a geração aguarda mais ditados, uma frase informa exatamente quantas palavras faltam. Ao atingir o marco, o botão fica habilitado e a tela informa que você já pode atualizar. A geração exige seu clique e confirmação, preservando as regras existentes do backend.

## Sua voz

A tela abre com um retrato curto, uma expressão recorrente, até três assuntos e até três hábitos de fala. Uma sugestão prática aparece quando existe no perfil. Preferências, vocabulário, medições de áudio e diagnóstico ficam sob divulgação progressiva. Perfis já salvos continuam legíveis.

O retrato por IA continua opcional e exige confirmação antes de cada geração. Estatísticas agregadas e termos filtrados são enviados pelo OpenRouter ao modelo `meta/muse-spark-1.3-contributor`; áudio e transcrições completas não fazem parte desse envio. A mudança visual não gera perfis automaticamente.

## Gravação sem voz

O PCM original é analisado localmente antes da normalização, persistência no histórico, envio aos provedores ou alteração do clipboard. Uma captura claramente silenciosa exibe apenas **Nenhuma voz encontrada** na barra, por 3,2 segundos. O arquivo temporário dessa captura vazia é encerrado e removido; gravações anteriores não são alteradas.

O detector é conservador: examina blocos de 10 ms, remove o componente DC e considera níveis máximos e contraste com o fundo. Não exige duração mínima, porcentagem mínima de fala ou média alta no áudio inteiro. Sons incertos são admitidos para transcrição. Assim, ruído mais alto pode ainda chegar ao provedor; esta verificação não é um reconhecedor de fala nem promete identificar todo ruído como silêncio.

O transporte legado Deepgram `streaming_final` agora abre após essa verificação, ao encerrar a captura. Isso mantém capturas silenciosas locais, com possível aumento da latência desse modo legado.

Os testes incluem silêncio, entrada constante, ruído muito baixo, sinais curtos em 16/44,1/48 kHz e os exemplos sintéticos “oi”, “sim” e “Bom dia, tudo bem?”, inclusive com volume reduzido e 30 segundos de pausas. Validação física com o microfone do usuário continua sendo uma categoria separada.

## Identidade e compatibilidade

- Interface, executável (`sonora.exe`), pacote, biblioteca Rust, instaladores, extensão, documentação e atribuição de requisições usam Sonora.
- `com.haumeavoice.app` permanece como identidade interna de armazenamento para preservar histórico, configurações e chaves DPAPI, sem migração do AppData.
- O identificador do host do navegador, a assinatura de backups portáteis e o mutex de instância única permanecem compatíveis com a geração anterior.
- Termos antigos do vocabulário do usuário são preservados. “Sonora” é uma dica para reconhecimento, sem forçar a capitalização do adjetivo em “trilha sonora”.
- A geração Electron é um projeto separado e não foi renomeada.

## Troca de aplicativo durante o ditado

O ditado é flexível: você pode começar a falar com foco em uma janela e alternar livremente para outra (por exemplo, abrir um chat e clicar no campo de mensagem). Ao concluir o processamento da transcrição, a simulação de colagem injeta o texto diretamente no campo atualmente ativo no sistema operacional.

## Instalação Windows

Após os testes e `npm run tauri build`, a atualização local da instalação antiga pode ser feita com PowerShell 7:

```powershell
pwsh -File scripts/install-windows.ps1 -InstallerPath src-tauri/target/release/bundle/nsis/Sonora_2.0.3_x64-setup.exe -BackupRoot F:\Dev-Backup
```

O procedimento exige um diretório de backup existente. Preserva o AppData e os executáveis anteriores, instala Sonora, verifica a versão e só então retira a instalação antiga. Confere hashes dos arquivos originais antes de reabrir. Reaponta inicialização automática e hosts de navegador que apontavam exatamente para a instalação antiga. Outros registros são preservados. A extensão precisa ser recarregada para aplicar seu código novo.

O instalador isolado instala Sonora em sua própria pasta; o procedimento acima também trata a substituição da instalação com o nome antigo. O backup contém dados privados e deve permanecer local. Para rollback, encerre Sonora e use o executável/instalador anterior preservado; não restaure dados antigos sobre ditados novos sem antes preservá-los.

## Verificação

Comandos: `npm run lint`, `npm run test:frontend`, `npm run test:ui`, `npm run build`, `cargo fmt --all -- --check`, `cargo test --locked`, `cargo clippy --locked --all-targets -- -D warnings`, `npm audit`, `cargo audit` e `npm run tauri build`.

As prévias com dados fictícios são exclusivas de desenvolvimento (`?insightsPreview`, `?gadgetPreview`) e não entram no bundle de produção. Os testes de interface usam mocks locais e não chamam provedores.

Qualificação local da 2.0.0 em 2026-09-05: 215 testes Rust, 18 testes de frontend e 12 testes de interface passaram, assim como lint, formatação, clippy e build. O teste nativo de identidade do campo, excluído da suíte automática por exigir uma janela interativa, passou separadamente com um campo local no Chrome e consultas em threads distintas. `npm audit` não encontrou vulnerabilidades; `cargo audit` manteve 17 avisos de dependências já existentes, sem atualizar pacotes fora do escopo.

Os instaladores NSIS e MSI foram gerados. A instalação local final via NSIS terminou com código 0, versão 2.0.0, backup e hashes idênticos dos 119 arquivos de dados existentes antes da reabertura. O nome e a versão também foram conferidos na interface instalada.

Na correção 2.0.1, passaram 14 testes de interface, 18 de frontend e 215 testes Rust, além de lint, formatação e build dos instaladores NSIS/MSI. A atualização local via NSIS terminou com código 0 e preservou os 128 arquivos de dados existentes. Versão, botão desabilitado e contagem de palavras restantes foram conferidos na interface instalada; a liberação do botão e a ausência de geração automática foram verificadas nos testes de interface com dados fictícios.

Não foram feitas chamadas de transcrição ou geração de perfil a provedores. O cenário completo com microfone físico, transcrição remota e colagem Chrome–Codex ainda requer validação de uso real.
