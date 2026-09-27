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
