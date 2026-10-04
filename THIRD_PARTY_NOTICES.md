# Third-Party Notices

The official service-status readers and outage-transition rules are adapted
from Pulse 1.7.2, commit `b570dd7`, Copyright (c) 2026 qunqin24,
Apache License 2.0: `Sources/Pulse/Providers/ServiceStatus.swift`,
`Sources/Pulse/Usage/OutageMemory.swift` and their tests. The Windows status
fixtures preserve only public page data from Pulse's 2026-10-04 captures.
Source: <https://github.com/qunqin24/Pulse>. Windows adds bounded reads,
shared request cooldown and an independent tray-notification setting.

The Hermes cumulative-session reader adapts `Sources/Pulse/Usage/Readers/HermesReader.swift`
from the same Pulse 1.7.2 commit, Copyright (c) 2026 qunqin24, Apache-2.0.
Windows adds bounded read-only SQLite snapshots, strict counters, named-profile
discovery and memory-only cache invalidation.

The Kiro Windows provider and ACP handshake are adapted from Pulse by
qunqin24, Copyright (c) 2026 qunqin24, under Apache License 2.0:
`Sources/Pulse/Providers/KiroUsageService.swift` and `KiroACPClient.swift` at
commit `3696a65b428272aa25c3ba611de8df2536983515`.
`windows/quotascope-core/tests/fixtures/upstream-kiro-pro-plus-usage.json` preserves
the upstream fixture. Source: <https://github.com/qunqin24/Pulse>.
The Apache-2.0 license is included in this repository's `LICENSE`.

The DeepSeek console protocol and balance fallback are adapted from Pulse by
qunqin24, Copyright (c) 2026 qunqin24, under Apache License 2.0:
`Sources/Pulse/Providers/DeepSeekConsole.swift`, `DeepSeekUsageService.swift`
and `Tests/PulseTests/DeepSeekConsoleTests.swift` at the same fixed commit above.
The Windows implementation uses its own DPAPI storage, bounded cache and tests.

The passive Codex rollout signal parser is adapted from Pulse's
`Sources/Pulse/Usage/CodexSignals.swift` and `Tests/PulseTests/CodexSignalsTests.swift`
at that fixed commit, Copyright (c) 2026 qunqin24, Apache License 2.0.
Windows adds conservative comparison and missing-data rules and stores daily
response counts instead of transcript text.

QuotaScope uses **HarmonyOS Sans SC**, Copyright 2021 Huawei Device Co., Ltd.
Unmodified Regular, Medium and Bold font files are bundled in `Fonts/` beside
the Windows executable. They are loaded privately by the application; no system
font installation is needed. The full HarmonyOS Sans Fonts License Agreement
is included in `Fonts/LICENSE.txt` and in
`windows/quotascope-win/assets/fonts/LICENSE.txt` in this repository.
Source: [Huawei design resources](https://developer.huawei.com/consumer/cn/design/resource/)
and [official font archive](https://developer.huawei.com/images/download/general/HarmonyOS-Sans.zip).

QuotaScope bundles provider marks derived from [Lobe Icons](https://github.com/lobehub/lobe-icons):

- `windows/quotascope-win/assets/icons/*.png`

Lobe Icons is distributed under the MIT License:

```text
MIT License

Copyright (c) 2023 LobeHub

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
```

The Windows app ships copies of the Lobe-derived provider marks, rasterised for its own renderer
(`windows/quotascope-win/assets/icons`). It is written in Rust against the
Microsoft Windows SDK via the [`windows`](https://github.com/microsoft/windows-rs)
crate, and links against further crates from crates.io — overwhelmingly
dual-licensed MIT OR Apache-2.0. The pinned list with versions is
`windows/Cargo.lock`, and each crate's repository carries its licence text;
the SQLite engine compiled into `rusqlite` (the `bundled` feature) is in the
public domain.

The Claude, OpenAI, Antigravity, Cursor, OpenCode, Kimi, Ollama, Z.ai, 智谱
and 清言, MiniMax, GitHub, Grok and xAI, and Volcengine names and marks remain
the property of their respective owners. Their inclusion identifies compatible
services and does not imply endorsement.

The Traditional Chinese, Japanese and Korean localization dictionaries and
provider quota fixtures in this update are adapted from
[Pulse](https://github.com/qunqin24/Pulse), commit `b396306`,
Copyright (c) 2026 qunqin24, under the Apache License, Version 2.0.
The full Apache license is included in `LICENSE`.

QuotaScope statically links the Zstandard decoder via zstd 0.14.0,
zstd-safe 8.0.0 and zstd-sys 2.1.0 (Zstandard 1.5.7). These components
are used under their BSD-3-Clause licences. No system-wide DLL or CLI
installation is required. Sources: https://github.com/gyscos/zstd-rs and
https://github.com/facebook/zstd. Exact dependency versions are pinned in
windows/Cargo.lock.

The zstd Rust wrapper, zstd-safe and zstd-sys carry this notice:

```
BSD 3-Clause License

Copyright (c) 2026, Alexandre Bury

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

```

The bundled Zstandard C library carries this notice:

```
BSD License

For Zstandard software

Copyright (c) Meta Platforms, Inc. and affiliates. All rights reserved.

Redistribution and use in source and binary forms, with or without modification,
are permitted provided that the following conditions are met:

 * Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

 * Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

 * Neither the name Facebook, nor Meta, nor the names of its contributors may
   be used to endorse or promote products derived from this software without
   specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR
ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON
ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

```
