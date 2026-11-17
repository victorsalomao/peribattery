# PeriBattery

Bateria dos periféricos sem fio na barra de tarefas do Windows 11, ao lado do indicador de idioma.
Um único `.exe` em Rust (~300 KB), sem software de fabricante, ~0,5 MB de working set e 0% de CPU entre leituras.

```
 [🎮 ⚡70%] [🎧 31%] [⌨ 92%] [🖱 10%]   POR  ⌃ 🔊 📶  14:32
                                        PTB           17/11
```

## Dispositivos

| Dispositivo | Como lê |
|---|---|
| ZOWIE EC1/EC2/EC3-CW | comando de leitura `A5 03` no receptor `04A5:8006` (cabo `8005` = carregando) |
| Logitech G915 TKL e outros LIGHTSPEED/Unifying/Bolt | HID++ 2.0 (`0x1004`, `0x1000`, `0x1001`) |
| Logitech G435 (dongle `046D:0ADF`) | protocolo próprio do dongle (CRC-16), em mV |
| DualSense / DualSense Edge | só lê o relatório de entrada (USB ou Bluetooth), nunca escreve |
| Controles Xbox | Windows.Gaming.Input (% exato), XInput como alternativa |
| 8BitDo (dongle 2.4 GHz) | aparece como conectado com "—": o dongle não informa o nível ao Windows |

Vermelho em ≤15%, raio quando carregando, esmaecido quando desligado (mostra o último valor).
Com jogo em tela cheia, as leituras ficam 5× mais espaçadas.

## Uso

- Primeira execução: copia-se para `%LOCALAPPDATA%\PeriBattery` e liga **Iniciar com o Windows**.
- Clique direito no widget: Atualizar agora, Iniciar com o Windows, Ocultar dispositivo, Sair.
- Diagnóstico: `peribattery.exe --list` (coleções HID) e `peribattery.exe --once` (uma leitura).

## Build

```bash
cargo test
cargo build --release
```

Requer Rust stable (MSVC) e as Build Tools do Visual Studio (rc.exe para ícone e manifesto).

## Desinstalar

Menu ➜ desmarque **Iniciar com o Windows** ➜ **Sair**, apague `%LOCALAPPDATA%\PeriBattery` e a chave `HKCU\Software\PeriBattery`.

## Créditos de protocolo

ZOWIE: github.com/yleshchuk/zowie-battery · G435: github.com/portyporty/g435-battery-percentage ·
HID++ e técnica do widget: github.com/renanleonel/windows-taskbar-battery (MIT).
