# emmu-snes

Wrapper WebAssembly do Emmu para **SNES / Super Famicom**. O núcleo é o [Super Sabicom](https://github.com/tanakh/super-sabicom), vendorizado como submódulo em `../emmu-super-sabicom`. No desktop, SNES roda no Mesen via `emmu-mesen-sys`; este crate atende o navegador.

## API

### Nativa (`SnesCore`)

- `new(rom: &[u8])` — prepara o cartucho, aplica reparos de cabeçalho e cria o `Context`.
- `game_title()` — título lido do cabeçalho.
- `set_button(button, pressed)` e `set_button_for_player(button, pressed, player)` — aceita `A`, `B`, `X`, `Y`, `L`, `R`, `Start`, `Select`, `Up`, `Down`, `Left`, `Right` para os jogadores 1 e 2.
- `run_frame()` — renderiza e devolve RGBA8888.
- `run_frame_without_rendering()` — clocka o console sem gerar frame.
- `audio_samples() -> Vec<i16>` — estéreo intercalado (esquerda/direita).
- `audio_sample_rate() -> u32`.
- `save_state()` / `load_state(&[u8])` — serialização bincode do estado do Super Sabicom.

### WASM (`SnesEmulator`)

Exporta `new`, `gameTitle`, `setButton`, `setButtonForPlayer`, `runFrame(renderGraphics)`, `audioSamples`, `audioSampleRate`, `saveState` e `loadState`.

## Vídeo e compatibilidade de ROM

- Frame de saída: **256 × 224**, RGBA8888.
- O framebuffer interno do core é esperado em pelo menos 512 × 448; a ponte amostra 1 pixel de cada 2.
- ROMs com tamanho não potência de 2 são preenchidas com zeros em memória, preservando cabeçalho de copiador de `0x200` bytes quando presente.
- `repair_overlong_lorom_title` e `repair_bsx_header` corrigem cabeçalhos LoROM/BS-X e recalculam checksum.
- Extensões aceitas pelo core: `.sfc`, `.smc`, `.swc`, `.fig` (a UI expõe `.sfc`, `.smc` e `.fig`).

## Árvore de arquivos

```text
emmu-snes/
├── .DS_Store
├── Cargo.toml              # cdylib + rlib; meru-interface, super-sabicom, wasm-bindgen
├── LICENSE                 # MIT, Copyright (c) 2026 EspetoRx
├── examples/
│   └── snes_smoke.rs       # 600 frames + teste de save/load
└── src/
    └── lib.rs              # reparos de ROM, SnesCore, SnesEmulator WASM, 4 testes
```

## Build, teste e WASM

```sh
cargo test -p emmu-snes

cargo run -p emmu-snes --example snes_smoke -- ROM.sfc [ROM.smc ...]
```

Pacote WASM usado pela UI:

```sh
cd crates/emmu-snes
wasm-pack build --target web \
  --out-dir ../../platforms/web/ui/src/wasm \
  --out-name emmu_snes
```

## Limitações

O Super Sabicom não implementa:

- PPU: BG Mode 6, Direct Color Mode e Interlaced Mode;
- chips de melhoria: Super FX, CX4, DSP, S-DD1, S-RTC e SA1.

Consequentemente, jogos que dependem desses recursos não funcionam no caminho WASM. No desktop, o Mesen pode ter compatibilidade diferente.

## Licenças

| Componente | Licença | Crédito |
| --- | --- | --- |
| `emmu-snes` | MIT | `Copyright (c) 2026 EspetoRx` — `LICENSE` |
| Super Sabicom | MIT | `Copyright (c) 2022 Hideyuki Tanaka` — `../emmu-super-sabicom/LICENSE` |
| `meru-interface` | MIT | interface `EmulatorCore`/`InputData` |
| `wasm-bindgen` | MIT OR Apache-2.0 | rustwasm |

Nenhuma ROM é distribuída com o crate.
