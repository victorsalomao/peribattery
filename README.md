# PeriBattery

Mostra a bateria do mouse, teclado, fone e controle sem fio na barra de tarefas do Windows 11, ao lado do indicador de idioma (POR/PTB).
É um único `.exe` de ~440 KB, sem instalar o software do fabricante. Usa ~1–3 MB de memória e quase nada de CPU, então não pesa no jogo.

```
 [🎮 ⚡70%] [🎧 31%] [⌨ 92%] [🖱 10%]   POR  ⌃ 🔊 📶  14:32
                                        PTB           17/11
```

- **Raio ⚡**: o dispositivo está carregando.
- **Vermelho**: 15% ou menos.
- **Apagado**: o dispositivo está desligado (mostra o último valor lido).
- **Passe o mouse em cima**: aparece só a informação daquele dispositivo, com a estimativa de tempo para acabar ou para encher.

## Instalar (para quem vai testar)

1. Baixe o `PeriBattery.exe` na página de [Releases](https://github.com/victorsalomao/peribattery/releases/latest).
2. Dê dois cliques nele.
3. O Windows vai mostrar **"O Windows protegeu o computador"**. Isso acontece porque o programa ainda não tem assinatura digital paga. Clique em **Mais informações** e depois em **Executar assim mesmo**.
4. Pronto. Ele se copia para `%LOCALAPPDATA%\PeriBattery`, aparece na barra de tarefas e passa a iniciar junto com o Windows. Não precisa de administrador.

Para atualizar, baixe o `.exe` novo e abra do mesmo jeito: ele substitui a versão instalada.

## O que funciona hoje

| Dispositivo | Situação |
|---|---|
| ZOWIE EC1/EC2/EC3-CW (receptor sem fio) | ✅ % e carregamento |
| Logitech com receptor LIGHTSPEED, Unifying ou Bolt (G915, G502 X, G Pro, MX…) | ✅ % e carregamento |
| Logitech G435 (dongle) | ✅ % |
| PS5 DualSense / DualSense Edge (USB ou Bluetooth) | ✅ % e carregamento |
| Controles Xbox e compatíveis (Bluetooth ou adaptador) | ✅ % (pela API de jogos do Windows) |
| 8BitDo com dongle 2.4 GHz | ⚠️ aparece conectado, mas sem % (o dongle não informa) |
| Razer, HyperX, Corsair, SteelSeries e outros | ❌ ainda não. Mande o relatório (abaixo) para entrar na lista. |

O app lê cada dispositivo a cada 5 s (o G435 a cada 15 s). Ele só envia comandos de **leitura** de bateria e nunca altera configurações dos periféricos.

## Menu (clique direito no widget)

- **Atualizar agora**: lê tudo de novo.
- **Iniciar com o Windows**: liga ou desliga a inicialização automática.
- **Ocultar**: esconde um dispositivo específico.
- **Relatório de dispositivos…**: abre no Bloco de Notas uma lista dos seus periféricos (fabricante, modelo e o que o PeriBattery está lendo). Não inclui nome de usuário, caminhos ou números de série.
- **Sair**.

## Algo não apareceu ou está errado?

1. Clique direito no widget ➜ **Relatório de dispositivos…**
2. Abra uma [issue](https://github.com/victorsalomao/peribattery/issues/new?template=dispositivo.md), cole o relatório e diga o modelo do seu mouse, teclado, fone e controle.

Se o widget nem aparecer, rode `PeriBattery.exe --report` (no Prompt de Comando, na pasta onde está o `.exe`).

## Desinstalar

**Configurações ➜ Aplicativos ➜ Aplicativos instalados ➜ PeriBattery ➜ Desinstalar.**
Isso fecha o app, tira da inicialização e apaga a pasta e as configurações.

## Para desenvolvedores

```bash
cargo test
cargo build --release
```

Requer Rust stable (MSVC) e as Build Tools do Visual Studio (rc.exe para ícone, manifesto e versão).
Diagnóstico: `--list` (coleções HID dos fabricantes suportados), `--once` (uma leitura), `--report`, `--bench`.
Registro de mudanças de leitura: `%LOCALAPPDATA%\PeriBattery\diag.log`.

## Créditos de protocolo

ZOWIE: github.com/yleshchuk/zowie-battery · G435: github.com/portyporty/g435-battery-percentage ·
HID++ e técnica do widget: github.com/renanleonel/windows-taskbar-battery (MIT).

## Licença

MIT. Veja [LICENSE](LICENSE).
