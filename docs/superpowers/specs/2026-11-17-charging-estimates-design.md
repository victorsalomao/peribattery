# Carregamento e estimativas de tempo (design curto, aprovado 2026-11-17)

## Intenção
Ver quando cada periférico está carregando (⚡) e, ao passar o mouse, quanto falta para
encher (carregando) ou para acabar (descarregando), para ter noção do uso de cada um.

## Detecção de carregamento
- G915 (HID++ 0x1001 flag 0x80), DualSense (nibble de estado), Xbox (WGI): já reportam.
- ZOWIE: PID 8005 (cabo) já marca. **Novo**: tendência de tensão.
- G435: **novo**: tendência de tensão.
- Tendência (por dispositivo): primeira leitura ≥ 4250 mV ⇒ carregando; descarregando e
  mV ≥ vale + 50 ⇒ carregando (pico = mV); carregando e mV ≤ pico − 20 ⇒ descarregando (vale = mV).

## Estimativas
- Histórico por dispositivo: (segundos, %) com espaçamento ≥ 300 s, máx. 288 amostras (24 h);
  zerado quando o estado carregando/descarregando muda ou o dispositivo desconecta.
- Ritmo = regressão linear (%/h). Válido se span ≥ 1200 s e |Δ%| ≥ 3.
- Descarga: restante = pct / ritmo. Carga: até 80% no ritmo medido; 80→100% na metade do ritmo.
- Sem medição: descarga usa ritmo aprendido salvo (HKCU\Software\PeriBattery\Rates) ou fábrica
  (ZOWIE 70 h, G915 40 h, G435 18 h, DualSense 6 h); carga aprendida ou "calculando…".
- Ritmo medido é salvo (média móvel 50/50) no máx. a cada 30 min por dispositivo.
- Tooltip: "Nome — 30% — descarregando · acaba em ~5 h 20 min (medido|aprendido|média de fábrica)".
  Minutos arredondados para 10 min acima de 1 h (menos redesenhos).

## Não-objetivos
Nenhuma leitura extra de hardware; sem gráfico; sem notificações.

## Abordagens consideradas
Ponta-a-ponta (2 pontos) — ruidoso com tensão; regressão linear — escolhida; filtro de Kalman — excessivo.
