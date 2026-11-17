# PeriBattery para a comunidade: distribuição, instalador e detecção genérica de dispositivos (avaliação)

Data: 2026-11-17 · Status: avaliação/recomendação (nada implementado) · Escopo: só pesquisa

## TL;DR

1. **Instalador:** não troque o `.exe` auto-instalável por MSI/MSIX agora. Em vez disso, transforme-o
   numa **instalação por usuário "de verdade"**: entrada em *Configurações → Apps* (chave `HKCU\…\Uninstall`),
   atalho no Menu Iniciar, `--uninstall`, recurso de versão no exe. Distribua por **GitHub Releases**
   (link, não anexo no Discord), com um passo a passo em PT-BR para o aviso do SmartScreen.
   Assim que houver releases públicos, peça assinatura grátis à **SignPath Foundation** (OSS, MIT) ou compre
   o **Certum Open Source Code Signing** (~€49–69/ano). Inno Setup fica como plano B caso o Defender
   implique com o auto-instalador. winget vem depois, quando estiver assinado.
2. **Detecção genérica:** monte um **registro de drivers** em ordem *específico do fabricante → genérico*,
   mais um **classificador de tipo** que junta as pistas por `ContainerId`. Fontes genéricas por ordem de
   custo/risco: propriedade de bateria Bluetooth do Windows (custo zero, sem I/O), *HID Battery Strength*
   (só GET), Windows.Gaming.Input/XInput, e depois GATT 0x180F. Para fabricantes, a ordem que mais rende
   para uma comunidade gamer brasileira: **DualShock 4 → família Compx/Kysona (Redragon, VXE/ATK, Attack Shark, Pulsar)
   → Razer → HyperX → SteelSeries → Corsair**.
3. **Comunidade:** item de menu **"Copiar relatório de diagnóstico"** e `--report` (texto sem dados pessoais
   para colar no Discord ou numa issue). Envio automático opt-in só depois, se valer a pena (LGPD: IP é dado pessoal).
4. **MVP para os amigos: ~2 dias.** Depois 1 semana para o registro/classificador e drivers sob demanda.

---

## 0. O que existe hoje (resumo do código)

- `main.rs`: `run_app()` copia o exe para `%LOCALAPPDATA%\PeriBattery\PeriBattery.exe`, liga o `Run` em HKCU
  na primeira vez e repassa a execução para a cópia instalada. Mutex `Local\PeriBattery`. Flags `--list`, `--once`,
  `--dump`, `--bench`.
- `settings.rs`: tudo em HKCU (`Software\PeriBattery`: `Installed`, `Hidden`), sem admin.
- `devices/mod.rs`: `enum Source { Zowie, Logi, G435, Ds, Pad }` fixo, sem registro. `hid::enumerate(&VIDS)`
  só enumera 5 VIDs (`046D, 04A5, 054C, 045E, 2DC8`).
- `model::DeviceKind` tem só `Mouse | Keyboard | Headset | Gamepad`. Não existe `Other`/`Unknown`.

Achados que influenciam este plano (vale conferir; **não alterei nada**):

- **`hidpp::kind_of`** mapeia todo tipo HID++ desconhecido para `Keyboard`. Pela feature `0x0005`
  função 2 (getDeviceType): 0 teclado, 1 controle remoto, 2 numpad, 3 mouse, 4 trackpad, 5 trackball,
  6 apresentador, 7 receptor, 8 headset, 9 webcam, 10 volante, 11 joystick, 12 gamepad, 13 dock,
  14 caixa de som, 15 microfone. Volantes, gamepads e caixas de som da Logitech apareceriam com ícone de teclado.
  É fácil de corrigir (10, 11, 12 → Gamepad; 14, 15 → Headset/Other).
- **`hid::enumerate`** tira o VID do caminho procurando `vid_`. Coleções HID por **Bluetooth** têm caminhos
  como `…{00001124-0000-1000-8000-00805f9b34fb}_VID&0002054c_PID&0ce6…` (clássico) ou
  `…{00001812-…}_Dev_VID&02xxxx_PID&…` (BLE), sem `vid_`. Então essas coleções provavelmente são **descartadas
  antes de abrir**. Se for isso, o DualSense por Bluetooth, que o README promete, não é detectado.
  Teste com o controle pareado por BT e `--list`. Para corrigir, aceite também `vid&0002`/`vid&02` ou use
  `HidD_GetAttributes`, que já é chamado em `caps`.
- O intervalo de leitura agora é 5 s (15 s no G435). Com dezenas de fontes genéricas, cada fonte vai precisar
  do próprio intervalo; ver §2.6.

---

## 1. Instalação e distribuição

### 1.1 O que os amigos vão ver (vale para qualquer opção não assinada)

Quando o arquivo vem de navegador, Discord, WhatsApp etc., o Windows grava a *Mark of the Web*
(`Zone.Identifier`). Na primeira execução o **SmartScreen** mostra **"O Windows protegeu o computador"**, com
só um botão "Não executar". É preciso clicar em **"Mais informações" → "Executar assim mesmo"**.

- Sem assinatura, a reputação do SmartScreen é **por hash do arquivo**. Cada versão nova volta à estaca zero.
- **Com assinatura** (OV ou via SignPath/Certum), a reputação acumula **no certificado**. Depois de algumas
  centenas ou milhares de execuções limpas, as versões futuras deixam de mostrar o aviso. EV **não** dá mais
  reputação instantânea (o SmartScreen mudou isso em 2024), então não vale o preço extra.
- Colocar dentro de um `.zip` não ajuda: o Explorer propaga a MOTW para os arquivos extraídos.
- **Defender (antivírus) é outro problema.** Um exe pequeno, não assinado, que **se copia para `%LOCALAPPDATA%`,
  grava `HKCU\…\Run` e se reabre** tem o perfil exato de um *dropper*. Os modelos de ML do Defender
  (detecções genéricas como `Wacatac`/`…!ml`) às vezes pegam binários Rust/C++ assim. Mitigações:
  recurso de versão (`VERSIONINFO`) com CompanyName/ProductName, não instalar em silêncio (mostrar algo na
  primeira execução), testar cada release no VirusTotal e, se houver falso positivo, enviar em
  `microsoft.com/wdsi/filesubmission` (costuma ser resolvido em 1 a 3 dias).

### 1.2 Comparação das opções

| Opção | Experiência do amigo | Esforço | Prós | Contras |
|---|---|---|---|---|
| **Exe auto-instalável (atual) + polimento** | baixa → SmartScreen → abre, já aparece na barra | ~0,5 dia | 300 KB, zero dependência, já funciona, sem UAC | heurística de "dropper"; desinstalação precisa ser escrita à mão |
| **Inno Setup** (per-user, `PrivilegesRequired=lowest`) | baixa `Setup.exe` → SmartScreen → assistente "Avançar" → pronto | ~0,5 dia + CI | padrão conhecido (o krisgfx9 usa), desinstalador e entrada em Apps grátis, menos suspeito para o Defender, licença livre (inclusive comercial) | +~1,5 MB; o instalador **também** precisa ser assinado; duas coisas para versionar |
| **WiX / MSI** | baixa `.msi` → SmartScreen → instala | 1–2 dias | ótimo para empresas e GPO | curva de aprendizado, MSI per-user é chato, nada que os amigos precisem |
| **MSIX (fora da Store)** | exige certificado confiável; com autoassinado o amigo teria que instalar o cert (inviável) | 2+ dias | instalação/remoção limpas | ruim sem assinatura; contêiner/virtualização de `LOCALAPPDATA`; startup via manifesto |
| **MSIX na Microsoft Store** | "Obter" na Store, sem aviso nenhum, atualização automática | 2–4 dias + certificação | **cadastro grátis para indivíduos desde set/2025**, e a Store **assina o MSIX de graça** | a certificação pode recusar um app que se injeta na `Shell_TrayWnd`; precisa de `runFullTrust`; o startup task do MSIX pode ser desligado pelo usuário/OS; revisão a cada versão |
| **winget** (`InstallerType: portable` ou `inno`) | `winget install PeriBattery` | ~2 h por versão (PR no `winget-pkgs`) | aceita exe não assinado (passa por varredura automática); evita o aviso do navegador | amigo leigo não usa terminal; precisa de releases públicos estáveis |
| **Scoop** (bucket próprio) | `scoop bucket add …; scoop install …` | ~1 h | trivial | nicho, só para quem já usa |

**Por usuário ou por máquina:** **por usuário**. O widget é por sessão de qualquer jeito (vive na barra de
tarefas do usuário), HKCU não pede UAC e o `Run` em HKCU só afeta quem instalou. Por máquina (Program Files +
HKLM) só faz sentido em PC compartilhado, e aí cada usuário teria que ligar o próprio autostart.

### 1.3 Assinatura de código: opções reais para um desenvolvedor no Brasil

| Opção | Custo | Disponível para pessoa física no BR? | Observações |
|---|---|---|---|
| **SignPath Foundation** | grátis | sim (é por projeto) | Exige repositório **público**, licença OSI (já é MIT), build no CI (GitHub Actions) e MFA. O certificado sai em nome de **"SignPath Foundation"** (o publisher exibido não é você) e exige uma linha de crédito no README. Aprovação manual, sem prazo garantido. |
| **Certum Open Source Code Signing** | ~€49 (nuvem/SimplySign) a ~€69/ano | sim, com verificação de identidade | Certificado em seu nome, chave na nuvem (SimplySign) ou no token. Desde fev/mar 2026 a validade máxima é 459 dias. Reputação SmartScreen acumula no cert. |
| **Azure Artifact Signing (ex-Trusted Signing)** | US$ 9,99/mês | **não**: pessoa física só EUA/Canadá; empresa só EUA/CA/UE/UK | Seria a melhor opção custo/benefício, mas não está disponível para o Victor hoje. |
| OV comercial (Sectigo/SSL.com/DigiCert) | ~US$ 200–400/ano + token HSM | sim | Desde 2023 a chave precisa estar em HSM/token. Caro para um projeto de amigos. |
| EV | US$ 300–600/ano | normalmente exige empresa | Não dá mais bypass do SmartScreen; não compensa. |
| Microsoft Store (MSIX) | grátis | sim | Só se o app passar na certificação (ver tabela acima). |

Ao assinar, use **timestamp** (`/tr http://timestamp…`). Sem ele, a assinatura "expira" junto com o certificado.

### 1.4 Atualização automática (GitHub Releases)

- **Checagem:** 1× por dia e na inicialização (com atraso de uns 5 min), **nunca em modo jogo**. Use um asset
  fixo, `https://github.com/<dono>/<repo>/releases/latest/download/latest.txt`, contendo
  `versão\nsha256\nurl`. Isso evita a API (limite de 60 req/h por IP sem token) e é uma linha só para ler.
- **HTTP sem dependência:** **WinHTTP** via `windows-sys` (`Win32_Networking_WinHttp`), carregado só na hora.
  Sem `reqwest`/`tokio`, então o binário e a RAM não mudam.
- **Aplicar:** baixa para `PeriBattery.new.exe` na pasta de instalação, confere o SHA-256 (e, quando assinado,
  `WinVerifyTrust` + nome do signatário), executa `PeriBattery.new.exe --apply-update`, que reaproveita a lógica
  atual (`stop_running_instance` + `copy_self_to`).
- **Padrão: avisar, não instalar sozinho.** Mostrar "Nova versão X disponível — clique para atualizar" no menu ou
  na dica. Um auto-updater é um ponto de ataque na cadeia de suprimentos: **2FA obrigatório na conta do GitHub**,
  e nunca baixar de uma URL que não seja a do próprio repositório.
- Arquivos baixados pelo próprio app não ganham MOTW, então a atualização não dispara o SmartScreen.

### 1.5 Entrada de desinstalação (o que falta no auto-instalador)

Grave em `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\PeriBattery`:
`DisplayName`, `DisplayVersion`, `Publisher`, `DisplayIcon` (= caminho do exe), `InstallLocation`,
`UninstallString` = `"…\PeriBattery.exe" --uninstall`, `QuietUninstallString`, `NoModify=1`, `NoRepair=1`,
`EstimatedSize` (KB, DWORD). Com isso o app aparece em *Configurações → Apps → Aplicativos instalados*.

O que `--uninstall` faz:
1. Fecha a instância em execução (`stop_running_instance`).
2. Apaga o valor `Run`, a chave `HKCU\Software\PeriBattery`, a chave `Uninstall` e o atalho do Menu Iniciar.
3. Remove o próprio exe. Um exe em execução não consegue se apagar, e `MOVEFILE_DELAY_UNTIL_REBOOT` exige admin.
   Opção simples: lançar `cmd /c ping -n 3 127.0.0.1 >nul & rmdir /s /q "<pasta>"` com `CREATE_NO_WINDOW`.
   Alternativa sem cmd: copiar-se para `%TEMP%` e rodar `--finish-uninstall <pasta>`.

Atalho no Menu Iniciar (`%APPDATA%\Microsoft\Windows\Start Menu\Programs\PeriBattery.lnk`, via `IShellLinkW`):
dá aos amigos um jeito de reabrir o app depois de "Sair". Hoje eles teriam que achar o exe em `%LOCALAPPDATA%`.

Primeira execução: mostrar um balão ou notificação ("PeriBattery instalado — aparece ao lado do relógio;
clique direito para opções"). Ajuda o usuário e também deixa a instalação menos parecida com malware silencioso.

### 1.6 Recomendação concreta

1. **Agora (MVP):** manter o exe único e acrescentar entrada Uninstall, `--uninstall`, atalho no Menu Iniciar,
   `VERSIONINFO` e aviso de primeira execução. Publicar em **GitHub Releases** com `PeriBattery.exe` +
   `PeriBattery.exe.sha256` e notas em PT-BR com print do "Mais informações → Executar assim mesmo".
   Mandar **o link** para os amigos no Discord, não o arquivo.
2. **Antes de cada release:** VirusTotal. Se o Defender marcar, enviar à Microsoft. Se continuar marcando,
   trocar para **Inno Setup per-user** (o instalador faz o "Run" e o app perde o comportamento de dropper).
3. **Em paralelo:** pedir a **SignPath Foundation** (grátis). Se demorar ou negarem, **Certum OSS** (~€50–70/ano).
4. **Depois de assinado:** manifesto **winget** (`portable`) e, opcionalmente, testar a **Store** com MSIX.

---

## 2. Identificação genérica de dispositivos

### 2.1 Princípio e regra de ouro

> **Nunca escrever um comando desconhecido em um dispositivo desconhecido.**
> Toda **escrita** (output report / SET_FEATURE) só acontece quando **VID:PID está numa allowlist do driver**
> **e** a coleção (usage page/usage/tamanhos de relatório) bate exatamente **e** o comando é uma
> **leitura documentada**. Caminhos genéricos são **somente leitura por construção**: propriedade do
> Windows, `GET_REPORT`/`GET_FEATURE` de um usage padrão, API do sistema. O caso ZOWIE `A5 09`
> (desemparelha o receptor) é o exemplo de por que isso não é negociável.

### 2.2 Fontes de bateria genéricas (sem código por modelo)

| Fonte | Como ler | Custo | Risco | Cobre |
|---|---|---|---|---|
| **Propriedade de bateria Bluetooth do Windows** `{104EA319-6EE2-4701-BD47-8DDBF425BBE5}, 2` (`DEVPROP_TYPE_BYTE`, 0–100) | `SetupDiGetClassDevsW(NULL, "BTHENUM"/"BTHLE", …, DIGCF_PRESENT\|DIGCF_ALLCLASSES)` + `SetupDiGetDevicePropertyW` | µs a poucos ms, **nenhum I/O de rádio** (o Windows já leu) | nulo | Fones BT (HFP), earbuds, mouses e teclados BLE: o mesmo valor que aparece em *Configurações → Bluetooth* |
| **HID Battery Strength** (Generic Device Controls `0x06`/usage `0x20`) e Battery System `0x85` (`0x65` AbsoluteStateOfCharge, `0x66` RemainingCapacity, `0x44` Charging) | `HidP_GetValueCaps` acha report ID e tipo; `HidD_GetFeature`/`HidD_GetInputReport` lê; `HidP_GetUsageValue` extrai | um GET por leitura | baixo (GET é leitura pela especificação HID) | Alguns teclados e mouses (Keychron, Apple, BLE com HOGP repassado), alguns controles. **Excluir** TLCs com page `0x84`/`0x85` como coleção principal (nobreaks/UPS). |
| **Windows.Gaming.Input** (`TryGetBatteryReport`) + **XInput** | já existe em `xbox.rs` | baixo (WinRT carregado sob demanda) | nulo | Controles Xbox, GameSir em modo GIP, vários controles "estilo Xbox" |
| **GATT Battery Service** `0x180F` / Battery Level `0x2A19` | WinRT `GattDeviceService` com `BluetoothCacheMode::Cached` | WinRT carregado: **+2–5 MB de working set**; leitura não cacheada gera tráfego de rádio | baixo | BLE que o Windows não reflete na propriedade acima. Usar **só** se a propriedade não existir e ler em cache. |
| **HID++ (Logitech)** | já existe | — | baixo (allowlist de receptores) | Lightspeed/Unifying/Bolt e também Logitech por Bluetooth (HID++ sobre BLE, PID do dispositivo) |
| **HeadsetControl** (`headsetcontrol -b -o json`) | subprocesso | criar processo: ~10–30 ms de CPU por leitura | médio (o próprio HeadsetControl manda comandos de fabricante, mas com curadoria) | Dezenas de headsets (SteelSeries Arctis, HyperX, Corsair, Logitech G, Audeze…) |

**HeadsetControl e GPL:** é GPL-3. **Chamar** um binário separado não "contamina" o PeriBattery (MIT).
**Incluir** o `.exe` no instalador ou no zip é "agregação": permitido, desde que vá junto o texto da GPL e a
oferta do código-fonte exato daquela versão (o krisgfx9 publica o fonte do HeadsetControl no release).
Recomendação: **não incluir no MVP**. Detectar se `headsetcontrol.exe` está ao lado do exe ou no `PATH`, ler a
cada ≥5 min, nunca em modo jogo, e documentar como "plugin opcional". A pegada fica em zero para quem não usa.

### 2.3 Classificador de tipo (mouse / teclado / fone / controle)

Junte todas as interfaces do mesmo aparelho pelo **`DEVPKEY_Device_ContainerId`**
`{8C7ED206-3F8A-4827-B3AB-AE9E1FAEFC6C}, 2`. Coleções HID, interface de áudio USB e nó Bluetooth do mesmo
dongle ou aparelho compartilham o container. Depois aplique as pistas **da mais forte para a mais fraca**;
a primeira conclusiva vence:

1. **O driver declarou** (HID++ `0x0005` getDeviceType; tabela de PIDs do driver Razer etc.).
2. **Bluetooth:** *Class of Device* `{2BD67D8B-8BEB-48D5-87E0-6CDA3428040A}, 4`. Major `0x04` = áudio → fone.
   Major `0x05` = periférico: minor bit `0x20` mouse, `0x10` teclado, bits baixos 1/2 joystick/gamepad.
   BLE normalmente não tem CoD; use a **categoria do container** (`Input.Keyboard`, `Input.Mouse`, `Input.Gaming`,
   `Audio.Headphones`, `Communication.Headset*`) ou o **GATT Appearance** (`0x03C1` teclado, `0x03C2` mouse,
   `0x03C3`/`0x03C4` joystick/gamepad, `0x0941` earbud, `0x0942` headset, `0x0943` headphones).
3. **Áudio no mesmo container → fone.** Barato e sem COM: o container tem uma interface USB `Class_01`
   (Audio) nos `CompatibleIds`. Mais preciso, com COM: `IMMDeviceEnumerator` + `PKEY_AudioEndpoint_FormFactor`
   `{1DA5D803-D492-4EDD-8C23-E0C0FFEE7F0E}, 0` (3 = Headphones, 5 = Headset). Faça só quando um dispositivo
   aparecer ou sumir, nunca no ciclo de leitura.
4. **Controle:** visto por WGI/XInput, ou TLC Generic Desktop `0x04` (Joystick) / `0x05` (Gamepad).
5. **TLCs HID Generic Desktop:** só `0x02` (Mouse) → mouse; só `0x06` (Keyboard) → teclado.
   **Cuidado:** quase todo mouse gamer expõe um TLC de teclado (macros) e quase todo teclado expõe um de mouse.
   Se os dois existem, **não decida** por aqui.
6. **Página Telephony `0x0B`** (ex. usage `0x05` Headset, Hook Switch, Phone Mute) → fone. A página Consumer
   `0x0C` (volume) é fraca demais, porque teclados também têm.
7. **Nome do produto** (`HidD_GetProductString`/FriendlyName) com palavras-chave em EN/PT/CN: "mouse",
   "keyboard/teclado", "headset/headphone/fone/buds", "controller/gamepad/controle". Por último, porque nomes de
   receptor ("USB Receiver", "2.4G Wireless") não dizem nada.
8. Se nada for conclusivo → **`Other`**: ícone genérico de bateria, e no menu **"Este dispositivo é um ▸
   Mouse / Teclado / Fone / Controle"**. A escolha fica salva em HKCU por `VID:PID` (ou endereço BT com hash).
   Isso resolve os casos ambíguos sem código e ainda gera dado útil para o relatório (§3).

Mudança de modelo necessária: `DeviceKind::Other` (e talvez `Earbuds`), mais um campo `via`
(`"2.4 GHz"`, `"Bluetooth"`, `"USB"`) para a dica.

### 2.4 Arquitetura proposta (registro de drivers)

```text
discovery (só em WM_DEVICECHANGE / boot / a cada 10 min):
    hid::enumerate_all()            todas as coleções HID (abrir com acesso 0 só para caps) + ContainerId
    bt::enumerate()                 nós BTHENUM/BTHLE + propriedades (bateria, CoD, conectado, nome)
    -> Inventory { containers: Vec<Container{ id, hid: Vec<HidInfo>, bt: Option<BtNode>, has_audio, … }> }

registry (ordem importa; o primeiro driver que "reivindica" um container fica com ele):
    1. vendor drivers   (allowlist VID:PID + coleção exata)  zowie, hidpp, g435, dualsense, ds4, razer, kysona, …
    2. system APIs      xbox (WGI/XInput)
    3. generic          bt_property, hid_battery_usage, gatt_battery (opcional), headsetcontrol (opcional)

trait Driver {
    fn key(&self) -> &'static str;
    fn claims(&self, c: &Container) -> bool;          // puro, sem I/O: só olha VID/PID/usages
    fn kind_hint(&self, c: &Container) -> Option<DeviceKind>;
    fn interval(&self, gaming: bool) -> Duration;     // G435 15 s, BT-prop 60 s, HeadsetControl 300 s …
    fn poll(&self, c: &Container) -> Vec<DeviceStatus>;
}
classifier::kind(container, driver_hint, user_override) -> DeviceKind
```

- `claims` é puro e roda só na descoberta, não a cada leitura. O ciclo de 5 s continua lendo só os containers
  com driver atribuído e handles/caminhos em cache.
- A enumeração completa de HID (todos os VIDs) custa alguns ms por evento de dispositivo. Aceitável, mas
  **não** a cada 5 s. A filtragem por VID atual vira um "pré-filtro" por driver.
- **O que aparece no widget:** só dispositivos **com fonte de bateria** (nível conhecido) ou que um driver marcou
  explicitamente como "conectado, nível indisponível" (caso 8BitDo). Aparelhos sem fonte (mouse com fio,
  receptor desconhecido) **não aparecem**. Ficam no relatório como "detectado, sem suporte" e num submenu
  opcional "Dispositivos não suportados…" que abre o relatório. Isso evita uma barra cheia de "—".
- **Desduplicação:** o mesmo aparelho pode chegar por dois caminhos (ex.: mouse Logitech BLE pelo HID++ e pela
  propriedade BT). Use o container como chave e a prioridade do registro para escolher a fonte.

### 2.5 Protocolos de fabricante: o que adicionar primeiro (comunidade gamer BR)

Fontes de *fatos de protocolo*: OpenRazer (GPL, só os fatos), HeadsetControl (GPL, fatos), Solaar,
rivalcfg, OpenRGB, OpenLinkHub, web drivers dos fabricantes (WebHID) e capturas USBPcap feitas pelo dono do
aparelho. **Não traduzir código do krisgfx9 (GPL-3) linha a linha.** Escreva uma nota de protocolo
(`docs/protocols/<fab>.md`) a partir das fontes e implemente a partir dela, citando a origem.

| Prioridade | Família | Modelos comuns no BR | Esforço | Risco | Notas |
|---|---|---|---|---|---|
| 1 | **Sony DualShock 4** | DS4 v1/v2 (`054C:05C4`, `09CC`), adaptador `0BA0` | 1–2 h | muito baixo | Igual ao DualSense, só leitura. USB: relatório `0x01`, byte 30: nibble baixo = nível 0–10 (0–11 no cabo), bit 4 = cabo. BT: relatório `0x11`, +2 bytes. Por BT o DS4 só manda relatório completo depois de um `GET_FEATURE 0x02`; a Steam normalmente já fez isso. Se não, aceitar "nível indisponível" em vez de forçar. |
| 2 | **Família Compx/Kysona** (protocolo compartilhado com checksum) | Redragon M916 Pro/M913, VXE/ATK R1/Dragonfly, Attack Shark (parte), Pulsar, HECATE, Taidu | 1 dia | médio | Maior retorno para a comunidade (mouses baratos e populares). Um protocolo cobre muitos VID:PID. Exige allowlist e validação de checksum na resposta. |
| 3 | **Razer** | Viper/DeathAdder V3 Pro, Basilisk, BlackShark V2 Pro | 1 dia | baixo-médio | Feature report de 90 bytes, comando classe `0x07` id `0x80` (bateria) e `0x84` (carregando), **transaction ID depende do PID**. Bem documentado (OpenRazer). Synapse aberto pode colidir. |
| 4 | **HyperX** | Cloud II/III Wireless, Cloud Flight, Cloud Alpha Wireless | 0,5–1 dia | médio | Por modelo, VIDs `0951` (Kingston) e `03F0` (HP pós-2021). Pulsefire sem protocolo público. HeadsetControl cobre vários. |
| 5 | **Areson (Redragon antigos)** | M915RGB-WL, M913 em `25A7:FA07/FA08` | 0,5 dia | médio | Feature report de 17 bytes (GetPower), passos de 10%. |
| 6 | **Attack Shark Xenta/Rongyuan** | X3, R1, X11, K86 | 1 dia | médio-alto | Xenta é **passivo** (ouvir broadcasts, sem escrever): ótimo para a regra de ouro, mas a leitura bloqueia até chegar um relatório; precisa de prazo fixo. |
| 7 | **SteelSeries** | Arctis Nova 7, Aerox | 0,5–1 dia | médio | Arctis bem documentado (HeadsetControl/rivalcfg). |
| 8 | **Corsair** | HS80, Void, Bragi (Slipstream) | 1–2 dias | alto | Bragi tem sessão/handshake; iCUE aberto conflita. Deixar para quando um amigo tiver o aparelho para testar. |
| — | Logitech headsets G733/G535/G Pro X | | 0,5 dia | baixo | São HID++ no PID do headset (não em receptor): ampliar a allowlist do `hidpp.rs`. |

Regra prática: **só implementar um driver quando alguém da comunidade tiver o aparelho e mandar o `--report`
+ o valor que o software do fabricante mostra.** Drivers "de documentação" sem teste são a maior fonte de
leitura errada.

### 2.6 Impacto em CPU/RAM (a prioridade do Victor)

- Fontes "propriedade do Windows" (BT, WGI) não fazem I/O no aparelho. Custo fica na casa dos µs/ms, igual ao atual.
- Cada driver tem seu próprio intervalo. Fontes caras (HeadsetControl, sessões tipo G435, GATT não cacheado)
  ficam em ≥60 s, e em modo jogo **pausam ou espaçam** (hoje o intervalo não muda em jogo; vale reavaliar com
  mais fontes).
- WinRT (GATT) e COM (MMDevice) carregam DLLs que ficam residentes. Usar só se a fonte barata falhar, e medir
  com o `--bench` existente antes de aceitar.
- Enumeração completa: só em `WM_DEVICECHANGE` (com debounce de ~2 s), no boot e na volta da suspensão.

---

## 3. Ângulo "comunidade"

### 3.1 `--report` + "Copiar relatório de diagnóstico" (MVP)

Os amigos não usam terminal (e o exe é subsistema `windows`, então `--report` só imprime se rodar de um cmd).
Por isso: **item de menu** que gera o texto, copia para a área de transferência e abre `report.txt` no Bloco de Notas.

Conteúdo sugerido:
```
PeriBattery 0.3.0 · Windows 11 26300 · x64 · taskbar: horizontal, primária, 125% DPI
[OK]   046D:C547 hidpp  "G915 TKL" tipo=teclado 64% (unified 0x1004)
[OK]   BT     bt-prop  "Fone BT #1" cod=0x240404 tipo=fone 80%
[SEM]  3554:F55D  coleções: FF02:0002 in/out/feat 0/0/17, 0001:0002 …  tipo?=mouse  sem driver
[ERRO] 1532:00B6 razer  timeout
```

**Privacidade:** **não** incluir endereço Bluetooth/MAC, números de série nem caminhos de instância (eles contêm
serial: `…\7&2a3b…`), nome do usuário ou do PC. Nomes Bluetooth podem ser pessoais ("Fone do João"): substitua por
`"Fone BT #1"` ou mostre só a categoria. VID:PID, usages e tamanhos de relatório não identificam a pessoa.

### 3.2 Envio automático opt-in (fase posterior)

- Só se a comunidade crescer além do Discord. **Desligado por padrão**, com tela mostrando *exatamente* o texto
  que será enviado e botão "Enviar este relatório" (envio manual, sem coleta contínua).
- Destino: GitHub Issue com formulário (o usuário cola o texto, sem servidor) ou um Cloudflare Worker gravando
  `VID:PID → driver/resultado`.
- **LGPD:** mesmo "anônimo", o IP do envio é dado pessoal. Base legal = consentimento, finalidade explícita
  ("ampliar suporte a dispositivos"), sem ID persistente do usuário, retenção curta, aviso no README.
  Mais simples: não guardar IP no backend e agregar por VID:PID.
- O dado mais valioso para crescer o suporte é "VID:PID + coleções + valor que o software do fabricante mostra".
  Isso exige o humano, então o fluxo manual (Discord/issue) rende mais que telemetria automática.

---

## 4. Roteiro em fases

### Fase 0: MVP para os amigos (~1,5–2 dias)

| Item | Esforço |
|---|---|
| Entrada Uninstall em HKCU + `--uninstall` + atalho no Menu Iniciar + `VERSIONINFO` + aviso de primeira execução | 0,5 dia |
| Fonte genérica **Bluetooth** (propriedade de bateria + CoD/categoria) e `DeviceKind::Other` | 0,5 dia |
| **DualShock 4** (só leitura) + correção do `kind_of` HID++ + verificar/corrigir VID em caminhos Bluetooth | 0,25 dia |
| "Copiar relatório de diagnóstico" / `--report` (sem dados pessoais) | 0,25 dia |
| Release no GitHub (exe + sha256 + guia PT-BR do SmartScreen) + VirusTotal; 2–3 amigos testam primeiro | 0,25 dia |

Fica de fora do MVP: auto-update, assinatura, novos fabricantes além do DS4.

### Fase 1: base genérica (~1 semana)
- Refatorar `Source` → registro `trait Driver` + `Inventory` por `ContainerId`; classificador §2.3; override do
  tipo pelo usuário.
- Fonte *HID Battery Strength* (só GET), com enumeração de todos os VIDs só em eventos de dispositivo.
- Detecção de fone por áudio no container.
- Auto-update "avisar e aplicar" via WinHTTP + SHA-256.
- Assinatura: SignPath (pedido) ou Certum; CI no GitHub Actions gerando release assinado.

### Fase 2: fabricantes sob demanda (contínuo, ~0,5–1 dia cada)
Ordem da §2.5, **puxada pelos relatórios** dos amigos. HeadsetControl como plugin opcional (não incluído).

### Fase 3: alcance
winget (`portable`), talvez a Microsoft Store (MSIX, assinatura grátis), envio opt-in de relatórios,
notificação de bateria fraca (toast) e suporte a Windows 10 / barra vertical / multi-monitor se os amigos pedirem.

### Riscos

| Risco | Prob. | Impacto | Mitigação |
|---|---|---|---|
| Defender marca o exe (heurística de dropper) | média | alto (amigos desistem) | VERSIONINFO, aviso de 1ª execução, VirusTotal por release, envio à MS, plano B Inno Setup, assinatura |
| Atrito do SmartScreen | certa (sem assinatura) | médio | guia com print; assinatura acumula reputação |
| Driver manda comando errado / desemparelha ou desconfigura aparelho | baixa com a regra | muito alto | allowlist VID:PID + coleção, só leituras documentadas, nenhum "probe", genéricos só GET |
| Leituras erradas em drivers não testados | média | médio | só lançar driver testado por alguém; marcar "experimental" na dica |
| Conflito com G HUB / Synapse / iCUE (respostas misturadas) | média | baixo-médio | casar respostas por SW-ID/transaction ID, descartar respostas que não batem, timeout curto |
| Widget quebra em configurações diferentes (Win10, barra vertical, ExplorerPatcher/StartAllBack, multi-monitor, atualização do Windows) | média-alta (mais usuários = mais variedade) | médio | fallback de ícone na bandeja (já previsto no design), relatório com info da barra |
| Regressão de CPU/RAM com fontes genéricas (WinRT/COM, enumeração completa) | média | alto para o Victor | enumerar só em evento; WinRT/COM só como último recurso; `--bench` no CI/checklist |
| Carga de suporte em cima do Victor | alta | médio | relatório copiável, issue template, FAQ em PT-BR |
| Contaminação GPL ao "se inspirar" no krisgfx9 | baixa | alto | implementar a partir de notas de protocolo citando fontes originais; HeadsetControl só como binário separado |
| Ataque de cadeia de suprimentos via auto-update | baixa | muito alto | 2FA, SHA-256 + assinatura, URL fixa do repositório, atualização com consentimento |

---

## 5. Perguntas em aberto para o Victor

1. Pode deixar o repositório **público** no GitHub? Releases, SignPath, winget e auto-update dependem disso.
2. Topa pagar **~€50–70/ano** (Certum, em seu nome) se a SignPath demorar? Ou prefere aceitar o aviso do SmartScreen por enquanto?
3. Quais aparelhos os amigos têm? Uma enquete rápida no Discord define a ordem da §2.5.
4. Dispositivos ambíguos: mostrar com ícone genérico e deixar o usuário escolher o tipo, ou esconder?
5. Precisa funcionar no **Windows 10** e/ou com barra vertical / ExplorerPatcher para alguém do grupo?
6. Envio automático de relatório (opt-in) interessa, ou o "copiar e colar no Discord" basta?
7. Atualização: só avisar (recomendado) ou atualizar sozinho em silêncio?

## Fontes consultadas

- Azure Artifact Signing (preço e elegibilidade): azure.microsoft.com/products/artifact-signing;
  learn.microsoft.com/answers/questions/2243504 e /5810735
- Certum Open Source Code Signing: shop.certum.eu/open-source-code-signing-on-simplysign.html, shop.certum.eu/code-signing.html
- SignPath Foundation: signpath.org/terms, about.signpath.io/product/open-source, docs.signpath.io/trusted-build-systems/github
- Microsoft Store grátis para indivíduos e assinatura de MSIX: blogs.windows.com/windowsdeveloper/2025/09/10/free-developer-registration-for-individual-developers-on-microsoft-store/,
  learn.microsoft.com/windows/apps/package-and-deploy/code-signing-options
- Referências locais (só leitura): krisgfx9/Wireless-Device-Battery-Monitor (README, CONTRIBUTING, `installer/WirelessBattery.iss`,
  fatos de `drivers/generic.cpp`), renanleonel/windows-taskbar-battery (`bluetooth.rs`, `headsetcontrol.rs`, MIT)
