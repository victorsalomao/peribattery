# PeriBattery — bateria dos periféricos na barra de tarefas (design)

Data: 2026-11-17 · Status: aguardando aprovação

## Intenção (o porquê)
Ver a bateria do mouse, teclado e fone direto na barra de tarefas do Windows 11, ao lado do
indicador de idioma ("POR PTB"), sem abrir software de fabricante, e **sem sentir nada enquanto
joga** (CPU ~0, RAM mínima, zero travadinha). Deve parecer nativo do Windows e iniciar sozinho com o PC.

## Objetivos
- Mostrar 4 dispositivos do Victor: **ZOWIE EC2-CW** (mouse), **Logitech G915 TKL** (teclado),
  **Logitech G435** (fone) e **DualSense de PS5** (USB hoje, Bluetooth depois), com ícone +
  porcentagem, no estilo do projeto krisgfx9.
- Estados visuais: normal; **vermelho** em ≤15%; **raio** quando carregando; **esmaecido** quando o
  dispositivo dorme/desliga (mantém o último valor conhecido).
- Iniciar com o Windows automaticamente, sem janela, sem console.
- Pegada alvo: **< 2 MB de RAM**, **0% de CPU entre leituras**, executável único < 1 MB.

## Não-objetivos
- Suporte a outras marcas/modelos (arquitetura permite adicionar depois, mas não agora).
- Bluetooth genérico, controles Xbox, HeadsetControl, notificação de bateria fraca por toast (fica para depois).
- Configurar DPI, RGB, sidetone ou qualquer coisa além de **ler** bateria.
- Instalador (é um .exe único; o próprio app registra o autostart).

## Anti-objetivos (falha mesmo funcionando)
- Qualquer queda de FPS, stutter ou latência de input em jogo.
- Enviar a um dispositivo qualquer comando que não seja leitura (o receptor Zowie **desemparelha** com `A5 09`).
- Interromper o áudio do G435 durante a leitura.

## Linguagem e abordagens consideradas
| Abordagem | RAM típica | Trade-off |
|---|---|---|
| **Rust + Win32 direto (escolhida)** | ~0,5–2 MB | Sem GC, sem runtime; mesma base provada pelo projeto renanleonel. Curva de aprendizado maior. |
| Go + syscalls Win32 | ~5–8 MB | GC e runtime sempre residentes; imperceptível, mas nunca chega ao piso do Rust. |
| C++ Win32 (como krisgfx9) | ~2 MB | Igual em performance ao Rust, porém sem segurança de memória. |

Victor escolheu **Rust**.

## Protocolos (o que cada dispositivo exige)
Os dois repositórios de referência já documentam os protocolos dos três aparelhos. Implementamos a
partir da **especificação do protocolo** (fontes MIT citadas), sem copiar código do krisgfx9 (GPL-3).

1. **ZOWIE EC2-CW** — receptor `04A5:8006` (cabo: `8005`). Coleção HID vendor (usage page ≥ `FF00`
   ou `00FF`), relatório de saída de 64 bytes sem ID. Envia `[00] A5 03`; resposta
   `[mV lo][mV hi][percent]…`. `mV = 0` → mouse desligado/fora de alcance. Rejeitar mV fora de 3000–4500.
   Se `percent` fora de 1..100, converter mV por curva (4200=100, 4000=75, 3890=50, 3760=25, 3680=10, 3500=0).
   Pelo cabo (`8005`) = carregando. Fonte: github.com/yleshchuk/zowie-battery.
   **Somente `A5 03` é enviado. Nunca outro comando.**
2. **Logitech G915 TKL** — via receptor LIGHTSPEED `046D:C547` (lista de PIDs de receptor do
   renanleonel). Protocolo **HID++ 2.0**: relatórios curto `0x10`/longo `0x11` nas coleções vendor
   `FF00`, índices de dispositivo 1..6; descobre o índice da feature pela Root (`0x0000`) e lê
   **`0x1004` Unified Battery** (fallback `0x1000` Battery Status, `0x1001` Voltage). Nome do
   dispositivo pela feature `0x0005`. Erro `0xFF`/`0x8F` = feature ausente/dispositivo dormindo.
3. **Logitech G435** — dongle `046D:0ADF`. **Não é HID++**: coleção `FF00:0001`, relatórios de 64 bytes
   com CRC-16/XMODEM (`cmd, len, crc lo, crc hi, body…`). Sessão: `84 01`, `85 01`, init, GET_MODE,
   SET_MODE 01, GET_MODE, GET_BATTERY (OTA `00 31`), fecha `85 02` (sempre, mesmo em erro).
   Resposta: padrão `00 07 00 06 <seq> 00 <mV lo> <mV hi>`. ~1,2 s por leitura, **não corta o áudio**
   neste dongle. Fonte: github.com/portyporty/g435-battery-percentage (MIT).
4. **Controle PS5 DualSense** — `054C:0CE6` (DualSense Edge `0DF2`). **Só leitura, nunca escreve.**
   Abre a coleção de gamepad em modo compartilhado (não atrapalha a Steam Input) e lê um relatório
   de entrada: **USB** = relatório `0x01` (64 bytes), byte de bateria no offset 53; **Bluetooth** =
   relatório `0x31`, mesmo campo deslocado em +1. Nibble baixo = nível 0–10 (→ `min(nível*10+5, 100)`),
   nibble alto = estado (0 descarregando, 1 carregando, 2 cheio). Precisão de 10%.
   Sem fio o DualSense usa **Bluetooth** (não Wi‑Fi); o mesmo código cobre os dois caminhos.
   Aparece no widget só enquanto conectado (USB ou Bluetooth); some ao desconectar.
5. **Controles estilo Xbox (feature "controles")** — via **Windows.Gaming.Input**
   (`Gamepad.Gamepads()` → `TryGetBatteryReport()`: `RemainingCapacityInMilliwattHours /
   FullChargeCapacityInMilliwattHours`, status Charging/Discharging/Idle/NotPresent), com
   **XInput** (`XInputGetBatteryInformation`, níveis vazio/baixo/médio/cheio) como fallback. Exato para
   controles Microsoft. Usa o crate `windows` só com a feature `Gaming_Input` (única exceção ao
   "só windows-sys"), carregado preguiçosamente na primeira vez que existir um gamepad.
   **8BitDo (VID `2DC8`)**: pelo dongle 2.4 GHz o controle se apresenta como "Xbox 360 wired"; XInput
   diz `wired` e WGI devolve sempre 1000/1000 (relato em HeyOkay/HaloBattery#110). Ou seja, **não há
   bateria real pelas APIs padrão**. Regra: para VID `2DC8`, ignorar o 100% fixo do WGI e mostrar o
   ícone de controle com "—" (conectado, nível não informado). Ler o nível real exige descobrir o
   protocolo do dongle capturando o tráfego do 8BitDo Ultimate Software (USBPcap) — tarefa de
   investigação separada, feita com o hardware em mãos. No dock o controle some do XInput: mostra
   esmaecido com o último estado.

## Arquitetura
```
main.rs          entrada; instância única (mutex nomeado); flags de debug --once / --list
ui/widget.rs     janela layered filha de Shell_TrayWnd, à esquerda da área de notificação
ui/render.rs     desenho per-pixel alpha (DIB 32bpp + UpdateLayeredWindow), fonte Segoe Fluent Icons
ui/tray.rs       fallback: ícone na bandeja se a barra estiver vertical/não encontrada
poller.rs        thread de leitura em prioridade baixa; envia resultado à UI via PostMessage
devices/hid.rs   enumeração e I/O HID com windows-sys (SetupAPI + HidD_* + ReadFile overlapped c/ timeout)
devices/zowie.rs, devices/hidpp.rs, devices/g435.rs, devices/dualsense.rs   um módulo por protocolo, funções puras testáveis
model.rs         DeviceStatus { kind, name, percent, mv, charging, connected, last_seen }
settings.rs      autostart (HKCU\...\Run) e preferências em HKCU\Software\PeriBattery
```
Fluxo: thread de poll acorda por timer **ou** evento (WM_DEVICECHANGE, retorno de suspensão) →
lê os 3 → se algo mudou, `PostMessage` para a UI → UI redesenha **apenas se mudou**.

## Pontos de melhoria em relação às referências (foco em performance)
- **Modo jogo**: `SHQueryUserNotificationState` detecta app em tela cheia/D3D exclusivo → intervalo de
  leitura sobe de 60 s para 5 min (a barra nem aparece em tela cheia; não faz sentido ler).
- **Thread em modo background** (`THREAD_MODE_BACKGROUND_BEGIN`): I/O e CPU com prioridade mínima,
  o escalonador nunca tira tempo do jogo.
- **Zero alocação no ciclo de leitura**: buffers fixos de 65 bytes reaproveitados; sem JSON, sem serde.
- **Sem `hidapi`**: só `windows-sys` (bindings sem custo). WinRT (crate `windows`) apenas para Windows.Gaming.Input, carregado só quando existe um controle estilo Xbox.
- **Redesenho só quando muda** e trim do working set (`SetProcessWorkingSetSize(-1,-1)`) após iniciar.
- Handles HID abertos e fechados por leitura (não segura o dispositivo), com timeout de 1 s em cada leitura.
- Intervalos: Zowie e G915 a cada 60 s; G435 a cada 120 s (sessão mais longa); DualSense a cada 60 s
  (lê um único relatório e fecha — não fica escutando o controle a 250 Hz). Imediato ao plugar/acordar.

## UI
```
 ... [🎮 ⚡70%] [🎧 82%] [⌨ 64%] [🖱 47%]   POR  ⌃ 🔊 📶  14:32
                                   PTB           17/11
```
Ícones Segoe Fluent Icons (headset/teclado/mouse), texto na cor do tema claro/escuro, segue DPI.
Hover: tooltip com nome completo, mV e "visto há X min". Clique direito: Atualizar agora,
Iniciar com o Windows ✓, Ocultar dispositivo ▸, Sair. Recria o widget se o Explorer reiniciar.

## Inicialização "nativa"
Na primeira execução: copia-se para `%LOCALAPPDATA%\PeriBattery\PeriBattery.exe` e grava
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. Subsistema `windows` (sem console),
manifesto com DPI per-monitor v2, ícone e versão embutidos. Sem admin.

## Riscos / pontos em aberto
- Não sabemos se o G915 TKL está no receptor `C547` ou em Bluetooth; o HID++ cobre o receptor.
  Se estiver em Bluetooth, adiciona-se leitura da propriedade de bateria do Windows (pequeno).
- Protocolos Zowie e G435 são engenharia reversa da comunidade; validaremos com `--once` no hardware real.

## Deixado ao implementador
Nomes internos, layout exato em pixels, mensagens do menu, testes unitários dos parsers/CRC/curvas.
