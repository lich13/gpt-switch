# Third-party notices

The circuit breaker defaults, state machine, queue policy, HTTP error classification and associated test cases in `src-tauri/src/gateway/` are adapted from cc-switch at commit `1ee2fdc3a791f1e73476c631c7ab7ce8fac0638f`.

Source: https://github.com/farion1231/cc-switch/tree/1ee2fdc3a791f1e73476c631c7ab7ce8fac0638f/src-tauri/src/proxy

Changes: single-lock state transitions with generation-tagged RAII permits; Retry-After; separate proxy circuits; strict per-request queue priority; raw-byte transport without cc-switch payload adapters.

MIT License

Copyright (c) 2025 Jason Young

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
## Scheduling reference

Provider slot accounting, bounded waiting and Responses WebSocket turn lifecycle were independently implemented in Rust with behavioral reference to Wei-Shaw/sub2api at a3eb7ef302961cba716dc78b39b93b60c467db0e (LGPL-3.0). No Go implementation or tests are copied.

https://github.com/Wei-Shaw/sub2api/tree/a3eb7ef302961cba716dc78b39b93b60c467db0e/backend/internal

## yawc 0.4.2

Unmodified WebSocket/deflate dependency, MPL-2.0. The full license is bundled in licenses/yawc-MPL-2.0.txt. Source is available at https://crates.io/api/v1/crates/yawc/0.4.2/download and https://github.com/infinitefield/yawc .


## Tray interaction reference

The separate quick panel, delayed blur handling and temporary macOS context-menu attachment follow the behavior of lich13/sub2api-ops-companion at 4d0eafc1e64385d1cdb6f6bd92eacb0b5c2e6f29. Adapted for gpt-Switch's window-scoped lifecycle and shared account/gateway state.

https://github.com/lich13/sub2api-ops-companion/tree/4d0eafc1e64385d1cdb6f6bd92eacb0b5c2e6f29/desktop


## Usage and pricing references

Usage and pricing UI behavior was independently implemented with behavioral reference to cc-switch commit `846de29c13ac4d65f164db8c15dd5fd58e29f972` (MIT). No source code is copied.

https://github.com/farion1231/cc-switch/tree/846de29c13ac4d65f164db8c15dd5fd58e29f972/src/components/usage

LiteLLM model price data is bundled from the public MIT-licensed repository at https://github.com/BerriAI/litellm . The bundled seed records its source URL, download timestamp and SHA-256; prices are used only for estimates.

models.dev model metadata and pricing are read from https://models.dev/api.json . The models.dev project is MIT licensed; its data remains attributed to that source.
