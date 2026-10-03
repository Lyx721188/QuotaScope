# Windows provider 读取路由

按当前 `Provider`、`Services` 注册表和设置入口核对：77 个内置 provider，77 个注册读取路由。新增十项的真实账号尚未逐一验证，细节见 [本轮移植状态](../upstream-implementation-status.md)。
“已实现”表示代码路由和解析测试存在，不能等同于每个账号都经过真实请求验证。
主账户可启用；API key/Cookie/JSON 凭据附加账户可添加，交互式附加账户 OAuth/login 仍未移植。

API key/Cookie header 由设置保存到 DPAPI 加密文件。浏览器导入仅在点击按钮时发生。
自建网关地址走共享验证；国际与中国大陆 storefront 各自使用独立 key。
旧的 macOS provider 文档只作上游行为参考，Windows 支持以此表与链接的 Rust 源码为准。

| Provider | raw id | Windows 路由 | 配置/来源 | 实现 |
|---|---|---|---|---|
| Claude Code | `claudeCode` | 已实现；需配置/登录 | CLI OAuth；Status Line 缓存、显式导入 Desktop/浏览器回退 | [claude_code.rs](../../windows/quotascope-core/src/providers/claude_code.rs) |
| Codex | `codex` | 已实现；需配置/登录 | CLI OAuth + app-server 回退；账户统计与重置次数读取 | [codex.rs](../../windows/quotascope-core/src/providers/codex.rs) |
| Kiro | `kiro` | 已实现；fixture/隔离 ACP 验证，真实账号未验证 | Kiro CLI 原有登录；设置启用/刷新，无需粘贴凭据 | [kiro.rs](../../windows/quotascope-core/src/providers/kiro.rs) · [配置与边界](windows-kiro.md) |
| Antigravity | `antigravity` | 已实现；需配置/登录 | 正在运行的本机语言服务器 | [antigravity.rs](../../windows/quotascope-core/src/providers/antigravity.rs) |
| Cursor | `cursor` | 已实现；需配置/登录 | Cursor 本机登录数据库 | [cursor.rs](../../windows/quotascope-core/src/providers/cursor.rs) |
| OpenCode Go | `openCodeGo` | 已实现；需配置/登录 | API key / 独立官网会话及实际账单 | [opencode.rs](../../windows/quotascope-core/src/providers/opencode.rs) |
| Kimi Code | `kimiCode` | 已实现；需配置/登录 | API key | [kimi.rs](../../windows/quotascope-core/src/providers/kimi.rs) |
| Ollama Cloud | `ollamaCloud` | 已实现；真实账号未验证 | 浏览器会话 / 只读官网额度页 | [remaining.rs](../../windows/quotascope-core/src/providers/remaining.rs) |
| z.ai | `zai` | 已实现；需配置/登录 | API key | [zai.rs](../../windows/quotascope-core/src/providers/zai.rs) |
| Zhipu | `glmCoding` | 已实现；需配置/登录 | API key / 本机已保存 key | [zai.rs](../../windows/quotascope-core/src/providers/zai.rs) |
| MiniMax | `minimax` | 已实现；需配置/登录 | API key | [minimax.rs](../../windows/quotascope-core/src/providers/minimax.rs) |
| MiniMax CN | `minimaxCN` | 已实现；需配置/登录 | API key | [minimax.rs](../../windows/quotascope-core/src/providers/minimax.rs) |
| GitHub Copilot | `copilot` | 已实现；需配置/登录 | GitHub device-code 登录 | [copilot.rs](../../windows/quotascope-core/src/providers/copilot.rs) |
| Grok | `grok` | 已实现；需配置/登录 | Grok 本机登录 | [grok.rs](../../windows/quotascope-core/src/providers/grok.rs) |
| Grok Bot | `grokBot` | 已实现；真实账号未验证 | Cursor 已保存登录 | [remaining.rs](../../windows/quotascope-core/src/providers/remaining.rs) |
| Volcengine | `volcengine` | 已实现；真实账号未验证 | AK:SK 签名 / arkcli | [remaining.rs](../../windows/quotascope-core/src/providers/remaining.rs) |
| Command Code | `commandCode` | 已实现；需配置/登录 | API key / CLI auth.json | [command_code.rs](../../windows/quotascope-core/src/providers/command_code.rs) |
| DeepSeek | `deepSeek` | 已实现；需配置/登录 | API key | [deepseek.rs](../../windows/quotascope-core/src/providers/deepseek.rs) |
| Devin | `devin` | 已实现；真实账号未验证 | 本机套餐缓存；未覆盖全部在线路线 | [remaining.rs](../../windows/quotascope-core/src/providers/remaining.rs) |
| Xiaomi Coding Plan | `xiaomiMiMo` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [xiaomi_mimo.rs](../../windows/quotascope-core/src/providers/xiaomi_mimo.rs) |
| sub2api | `sub2api` | 已实现；需配置/登录 | 地址 + API key | [sub2api.rs](../../windows/quotascope-core/src/providers/sub2api.rs) |
| New API | `newAPI` | 已实现；需配置/登录 | 地址 + API key | [new_api.rs](../../windows/quotascope-core/src/providers/new_api.rs) |
| V2EX | `v2ex` | 已实现；需配置/登录 | API key | [v2ex.rs](../../windows/quotascope-core/src/providers/v2ex.rs) |
| Qoder | `qoder` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [qoder.rs](../../windows/quotascope-core/src/providers/qoder.rs) |
| StepFun | `stepFun` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [step_fun.rs](../../windows/quotascope-core/src/providers/step_fun.rs) |
| ClinePass | `clinePass` | 已实现；需配置/登录 | API key | [cline_pass.rs](../../windows/quotascope-core/src/providers/cline_pass.rs) |
| Alibaba Coding Plan | `alibabaCodingPlan` | 已实现；需配置/登录 | API key | [alibaba_coding_plan.rs](../../windows/quotascope-core/src/providers/alibaba_coding_plan.rs) |
| Alibaba Token Plan | `alibabaTokenPlan` | 已实现；真实账号未验证 | 百炼 CLI (bl) 已保存登录 | [remaining.rs](../../windows/quotascope-core/src/providers/remaining.rs) |
| Qwen Cloud | `qwenCloud` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [qwen_cloud.rs](../../windows/quotascope-core/src/providers/qwen_cloud.rs) |
| Factory | `factory` | 已实现；需配置/登录 | API key | [factory.rs](../../windows/quotascope-core/src/providers/factory.rs) |
| Gemini | `gemini` | 已实现；真实账号未验证 | Gemini CLI Google 登录 | [remaining.rs](../../windows/quotascope-core/src/providers/remaining.rs) |
| Kilo Code | `kiloCode` | 已实现；需配置/登录 | API key | [kilo_code.rs](../../windows/quotascope-core/src/providers/kilo_code.rs) |
| Augment Code | `augment` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [augment.rs](../../windows/quotascope-core/src/providers/augment.rs) |
| JetBrains AI | `jetBrainsAI` | 未移植 | 未实现 | — |
| T3 Chat | `t3Chat` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [t3chat.rs](../../windows/quotascope-core/src/providers/t3chat.rs) |
| Synthetic | `synthetic` | 已实现；需配置/登录 | API key | [synthetic.rs](../../windows/quotascope-core/src/providers/synthetic.rs) |
| ElevenLabs | `elevenLabs` | 已实现；需配置/登录 | API key | [elevenlabs.rs](../../windows/quotascope-core/src/providers/elevenlabs.rs) |
| Warp | `warp` | 已实现；需配置/登录 | API key | [warp.rs](../../windows/quotascope-core/src/providers/warp.rs) |
| Windsurf | `windsurf` | 已实现；真实账号未验证 | 导入 Chromium localStorage / 会话 JSON | [remaining.rs](../../windows/quotascope-core/src/providers/remaining.rs) |
| Bifrost | `bifrost` | 已实现；需配置/登录 | 地址 + API key | [bifrost.rs](../../windows/quotascope-core/src/providers/bifrost.rs) |
| Chutes | `chutes` | 已实现；需配置/登录 | API key | [chutes.rs](../../windows/quotascope-core/src/providers/chutes.rs) |
| LongCat | `longCat` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [longcat.rs](../../windows/quotascope-core/src/providers/longcat.rs) |
| ZoomMate | `zoomMate` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [zoom_mate.rs](../../windows/quotascope-core/src/providers/zoom_mate.rs) |
| Notion AI | `notionAI` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [notion_ai.rs](../../windows/quotascope-core/src/providers/notion_ai.rs) |
| IBM Bob | `ibmBob` | 已实现；需配置/登录 | API key | [ibm_bob.rs](../../windows/quotascope-core/src/providers/ibm_bob.rs) |
| Nous Portal | `nousPortal` | 已实现；真实账号未验证 | Hermes 已保存 OAuth；不代为续期 | [remaining.rs](../../windows/quotascope-core/src/providers/remaining.rs) |
| Raycast AI | `raycastAI` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [raycast_ai.rs](../../windows/quotascope-core/src/providers/raycast_ai.rs) |
| GitKraken AI | `gitKraken` | 已实现；需配置/登录 | API key | [gitkraken.rs](../../windows/quotascope-core/src/providers/gitkraken.rs) |
| xKiro | `xKiro` | 已实现；需配置/登录 | API key | [xkiro.rs](../../windows/quotascope-core/src/providers/xkiro.rs) |
| Abacus AI | `abacus` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [abacus.rs](../../windows/quotascope-core/src/providers/abacus.rs) |
| Moonshot | `moonshot` | 已实现；需配置/登录 | API key | [moonshot.rs](../../windows/quotascope-core/src/providers/moonshot.rs) |
| Hyper | `hyper` | 已实现；需配置/登录 | API key | [hyper.rs](../../windows/quotascope-core/src/providers/hyper.rs) |
| Atlas Cloud | `atlasCloud` | 已实现；需配置/登录 | API key | [atlas_cloud.rs](../../windows/quotascope-core/src/providers/atlas_cloud.rs) |
| Poe | `poe` | 已实现；需配置/登录 | API key | [poe.rs](../../windows/quotascope-core/src/providers/poe.rs) |
| Venice | `venice` | 已实现；需配置/登录 | API key | [venice.rs](../../windows/quotascope-core/src/providers/venice.rs) |
| OpenAI API | `openAIPlatform` | 已实现；需配置/登录 | API key | [openai_platform.rs](../../windows/quotascope-core/src/providers/openai_platform.rs) |
| Amp | `amp` | 已实现；需配置/登录 | API key | [amp.rs](../../windows/quotascope-core/src/providers/amp.rs) |
| Zed | `zed` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [zed.rs](../../windows/quotascope-core/src/providers/zed.rs) |
| Sakana AI | `sakana` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [sakana.rs](../../windows/quotascope-core/src/providers/sakana.rs) |
| Mistral | `mistral` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [mistral.rs](../../windows/quotascope-core/src/providers/mistral.rs) |
| Codebuff | `codebuff` | 已实现；需配置/登录 | API key | [codebuff.rs](../../windows/quotascope-core/src/providers/codebuff.rs) |
| LLM API Key Proxy | `llmProxy` | 已实现；需配置/登录 | 地址 + API key | [llm_proxy.rs](../../windows/quotascope-core/src/providers/llm_proxy.rs) |
| LiteLLM | `liteLLM` | 已实现；需配置/登录 | 地址 + API key | [litellm.rs](../../windows/quotascope-core/src/providers/litellm.rs) |
| Aixy | `aixy` | 已实现；需配置/登录 | API key | [aixy.rs](../../windows/quotascope-core/src/providers/aixy.rs) |
| Neuralwatt | `neuralwatt` | 已实现；需配置/登录 | API key | [neuralwatt.rs](../../windows/quotascope-core/src/providers/neuralwatt.rs) |
| ClawRouter | `clawRouter` | 已实现；需配置/登录 | API key | [claw_router.rs](../../windows/quotascope-core/src/providers/claw_router.rs) |
| ZenMux | `zenMux` | 已实现；需配置/登录 | API key | [zenmux.rs](../../windows/quotascope-core/src/providers/zenmux.rs) |
| v0 | `v0` | 已实现；需配置/登录 | API key | [v0.rs](../../windows/quotascope-core/src/providers/v0.rs) |
| DevPass | `devPass` | 已实现；需配置/登录 | API key | [devpass.rs](../../windows/quotascope-core/src/providers/devpass.rs) |
| Perplexity | `perplexity` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [perplexity.rs](../../windows/quotascope-core/src/providers/perplexity.rs) |
| Manus | `manus` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [manus.rs](../../windows/quotascope-core/src/providers/manus.rs) |
| Hugging Face | `huggingFace` | 已实现；需配置/登录 | API key | [huggingface.rs](../../windows/quotascope-core/src/providers/huggingface.rs) |
| DeepInfra | `deepInfra` | 已实现；需配置/登录 | API key | [deepinfra.rs](../../windows/quotascope-core/src/providers/deepinfra.rs) |
| xAI API | `xaiAPI` | 已实现；需配置/登录 | API key | [xaiapi.rs](../../windows/quotascope-core/src/providers/xaiapi.rs) |
| Replicate | `replicate` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [replicate.rs](../../windows/quotascope-core/src/providers/replicate.rs) |
| TypeSafe | `typeSafe` | 已实现；需配置/登录 | 浏览器导入 / Cookie header | [type_safe.rs](../../windows/quotascope-core/src/providers/type_safe.rs) |
| Vercel AI Gateway | `vercelAIGateway` | 已实现；需配置/登录 | API key | [vercel_ai_gateway.rs](../../windows/quotascope-core/src/providers/vercel_ai_gateway.rs) |

新增十项已注册路由与样本解析，仍需真实账号验收；来源/回退的限制以本轮移植状态为准。
扩展程序独立于这 77 个内置 provider，见 [Windows extensions](../extensions.md)。

托盘用量页使用上游明确的 18 个 URL；只有启用的账户出现在托盘中。
详细卡历史支持主账户 Claude Code/Codex 本机账本、z.ai/智谱服务器统计及 OpenCode 官网实际请求账单。
操作步骤和验证边界见 [Windows 1.2 guide](../windows-1.2.md)。
