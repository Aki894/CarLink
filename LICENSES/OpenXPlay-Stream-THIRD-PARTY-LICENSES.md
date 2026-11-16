# Third-Party Licenses

Third-party software used by distributed builds of OpenXPlay-Stream. Keep this file with every
distributed build.

| Scope | Component | Version | License |
| --- | --- | --- | --- |
| runtime | `org.jetbrains.kotlin:kotlin-stdlib` | 2.2.10 | Apache-2.0 |
| runtime | `org.jetbrains:annotations` (via kotlin-stdlib) | 13.0 | Apache-2.0 |
| runtime (web-server, clink) | `org.jetbrains.kotlinx:kotlinx-serialization-json` and `-core` | 1.9.0 | Apache-2.0 |
| build | Gradle Wrapper (`kotlin/gradlew`, `kotlin/gradle/wrapper/`) | 9.5.0 | Apache-2.0 |
| build (web) | `typescript` | 5.9.3 | Apache-2.0 |
| build (web) | `vite`, including helpers it may emit into the browser bundle | 7.1.12 | MIT |
| test | `org.jetbrains.kotlin:kotlin-test-junit5` | 2.2.10 | Apache-2.0 |
| test | `org.junit.jupiter`, `org.junit.platform` | 5.13.4 / 1.13.4 | EPL-2.0 |
| runtime (kotlin/web-receiver, rust/web-receiver) | Lucide icons | 1.57.0 | ISC; Feather-derived icons: MIT |
| test (web) | `vitest` | 3.2.4 | MIT |

None of the runtime dependencies above ships an Apache-2.0 NOTICE file.

Test clips under `kotlin/core/src/test/resources/media` and the placeholder clip in
`kotlin/clink/src/main/resources` were generated for this project with FFmpeg; no third-party code is
included in them.

Lucide icons v1.57.0 — ISC License, Copyright (c) Lucide Icons and Contributors;
icons derived from Feather (here: x, maximize, minimize) — MIT License,
Copyright (c) 2013-present Cole Bemis; https://github.com/lucide-icons/lucide;
embedded as inline SVG markup in kotlin/web-receiver/receiver and rust/web-receiver/receiver.

ISC License

Copyright (c) 2026 Lucide Icons and Contributors

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.


The MIT License (MIT) (for the icons listed above)

Copyright (c) 2013-present Cole Bemis

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
