//! Two-language string table (English and Simplified Chinese), read at
//! display time so text follows the language setting rather than freezing at
//! whichever language was current when the reading was taken.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    Chinese,
    TraditionalChinese,
    Japanese,
    Korean,
}

static LANGUAGE: AtomicU8 = AtomicU8::new(0);

pub fn set_language(language: Language) {
    LANGUAGE.store(
        match language {
            Language::English => 0,
            Language::Chinese => 1,
            Language::TraditionalChinese => 2,
            Language::Japanese => 3,
            Language::Korean => 4,
        },
        Ordering::Relaxed,
    );
}

pub fn current() -> Language {
    match LANGUAGE.load(Ordering::Relaxed) {
        1 => Language::Chinese,
        2 => Language::TraditionalChinese,
        3 => Language::Japanese,
        4 => Language::Korean,
        _ => Language::English,
    }
}

/// Detect from the Windows UI language, before settings are read.
///
/// The environment variables are the fallback: on Windows they are usually
/// unset, and the system's display language lives in kernel32.
#[cfg(windows)]
pub fn detect_from_system() {
    use windows::Win32::Globalization::GetUserDefaultUILanguage;
    // PRIMARYLANGID: the low 10 bits; 0x04 is Chinese. This machine's
    // answer covers zh-CN, zh-TW and every other variant.
    let id = unsafe { GetUserDefaultUILanguage() };
    set_language(match id & 0x3FF {
        0x04 if matches!(id, 0x0404 | 0x0c04 | 0x1404) => Language::TraditionalChinese,
        0x04 => Language::Chinese,
        0x11 => Language::Japanese,
        0x12 => Language::Korean,
        _ => Language::English,
    });
}

#[cfg(not(windows))]
pub fn detect_from_system() {
    let zh = std::env::var("SYSTEM_LANGUAGE").is_ok_and(|v| v.to_lowercase().starts_with("zh"))
        || std::env::var("LANG").is_ok_and(|v| v.to_lowercase().starts_with("zh"));
    set_language(if zh {
        Language::Chinese
    } else {
        Language::English
    });
}

struct Entry {
    key: &'static str,
    en: &'static str,
    zh: &'static str,
}

const TABLE: &[Entry] = &[
    Entry { key: "Sign in to the provider's CLI first.", en: "Sign in to the provider's CLI first.", zh: "请先在服务商的 CLI 中登录。" },
    Entry { key: "The local login expired. Sign in to the provider's CLI again.", en: "The local login expired. Sign in to the provider's CLI again.", zh: "本机登录已过期，请重新登录服务商的 CLI。" },
    Entry { key: "No saved quota found in the provider's local app.", en: "No saved quota found in the provider's local app.", zh: "服务商的本机应用尚未保存额度读数。" },
    Entry { key: "Provider request bills · all machines · actual recorded USD cost", en: "Provider request bills · all machines · actual recorded USD cost", zh: "服务商请求账单 · 所有设备 · 实际记录的美元费用" },
    Entry { key: "The minimum cache window has ended; entries may remain cached.", en: "The minimum cache window has ended; entries may remain cached.", zh: "已超过最短缓存复用期，条目仍可能留在缓存中。" },
    Entry { key: "Uses kiro-cli's existing login. Sign in with kiro-cli first.", en: "Uses kiro-cli's existing login. Sign in with kiro-cli first.", zh: "使用 kiro-cli 已保存的登录；请先在 kiro-cli 中登录。" },
    Entry { key: "Reads the plan Devin saved on this PC. Open Devin and sign in first.", en: "Reads the plan Devin saved on this PC. Open Devin and sign in first.", zh: "读取 Devin 在本机保存的套餐。请先打开 Devin 并登录。" },
    Entry { key: "Uses Cursor's existing login to read the Grok Bot allowance.", en: "Uses Cursor's existing login to read the Grok Bot allowance.", zh: "使用 Cursor 已保存的登录读取 Grok Bot 额度。" },
    Entry { key: "Uses Bailian CLI (bl)'s existing login, for international and mainland plans.", en: "Uses Bailian CLI (bl)'s existing login, for international and mainland plans.", zh: "使用百炼 CLI（bl）的登录读取国际与大陆套餐。" },
    Entry { key: "Uses Gemini CLI's Google login; API key and Vertex AI modes do not report this allowance.", en: "Uses Gemini CLI's Google login; API key and Vertex AI modes do not report this allowance.", zh: "使用 Gemini CLI 的 Google 登录；API key 和 Vertex AI 模式不报告此项额度。" },
    Entry { key: "Reads the quota JetBrains AI saved on this PC. Open the IDE to update it.", en: "Reads the quota JetBrains AI saved on this PC. Open the IDE to update it.", zh: "读取 JetBrains AI 在本机保存的额度；打开 IDE 可更新读数。" },
    Entry { key: "Uses the Nous Portal login Hermes saved. Run Hermes to renew an expired login.", en: "Uses the Nous Portal login Hermes saved. Run Hermes to renew an expired login.", zh: "使用 Hermes 保存的 Nous Portal 登录；登录过期后请运行 Hermes 更新。" },
    Entry { key: "Dashboard", en: "Dashboard", zh: "用量概览" },
    Entry { key: "Connection source", en: "Connection source", zh: "读取来源" },
    Entry { key: "Cached reading", en: "Cached reading", zh: "缓存读数" },
    Entry { key: "Reading age", en: "Reading age", zh: "读数距今" },
    Entry { key: "Add account", en: "Add account", zh: "添加账户" },
    Entry { key: "Remove account", en: "Remove account", zh: "移除账户" },
    Entry { key: "Enter a valid credential for the additional account.", en: "Enter a valid credential for the additional account.", zh: "请为附加账户输入有效凭据。" },
    Entry { key: "For an additional Claude account, paste exported .credentials.json. Local usage belongs to the primary account only.", en: "For an additional Claude account, paste exported .credentials.json. Local usage belongs to the primary account only.", zh: "添加 Claude 账户时粘贴导出的 .credentials.json。本机用量只归属主账户。" },
    Entry { key: "Paste a different account credential above, then add it. The primary credential stays unchanged.", en: "Paste a different account credential above, then add it. The primary credential stays unchanged.", zh: "在上方粘贴另一个账户的凭据，再点击添加。主账户凭据会保留。" },
    Entry { key: "Import Claude Desktop / browser fallback", en: "Import Claude Desktop / browser fallback", zh: "导入 Claude Desktop / 浏览器回退会话" },
    Entry { key: "Forget fallback session", en: "Forget fallback session", zh: "清除回退会话" },
    Entry { key: "Configure quotascope --statusline in Claude to use reported rate limits as a 15-minute cached fallback.", en: "Configure quotascope --statusline in Claude to use reported rate limits as a 15-minute cached fallback.", zh: "在 Claude 状态栏中配置 quotascope --statusline，可将报告的额度作为 15 分钟内有效的缓存回退。" },
    Entry { key: "Import the OpenCode console session to read Go quotas and actual request bills across machines. The API key remains separate.", en: "Import the OpenCode console session to read Go quotas and actual request bills across machines. The API key remains separate.", zh: "导入 OpenCode 官网会话，可读取 Go 额度和各设备的实际请求账单；API key 独立保存。" },
    Entry { key: "Forget console session", en: "Forget console session", zh: "清除官网会话" },
    Entry { key: "Import the Windsurf website's localStorage session, or paste its four-field JSON bundle.", en: "Import the Windsurf website's localStorage session, or paste its four-field JSON bundle.", zh: "从浏览器导入 Windsurf 网站会话，或粘贴包含四项凭据的 JSON。" },
    Entry { key: "Save proxy and refresh", en: "Save proxy and refresh", zh: "保存代理并刷新" },
    Entry { key: "Proxy mode", en: "Proxy mode", zh: "代理模式" },
    Entry { key: "System proxy", en: "System proxy", zh: "系统代理" },
    Entry { key: "No proxy", en: "No proxy", zh: "不使用代理" },
    Entry { key: "Manual proxy", en: "Manual proxy", zh: "手动代理" },
    Entry { key: "HTTP proxy (empty uses system settings)", en: "HTTP proxy (empty uses system settings)", zh: "HTTP 代理（留空则使用系统设置）" },
    Entry { key: "Global shortcuts: Ctrl+Shift+F10 panel, F11 dashboard, F12 refresh", en: "Global shortcuts: Ctrl+Shift+F10 panel, F11 dashboard, F12 refresh", zh: "全局快捷键：Ctrl+Shift+F10 面板，F11 用量概览，F12 刷新" },
    Entry { key: "Animated dashboard buddy", en: "Animated dashboard buddy", zh: "用量概览中的动画伙伴" },
    Entry { key: "Language", en: "Language", zh: "语言" },
    Entry { key: "Automatic", en: "Automatic", zh: "自动" },
    Entry { key: "Bottom", en: "Bottom", zh: "底部" },
    Entry { key: "Free vertical", en: "Free vertical", zh: "自由摆放（竖向）" },
    Entry { key: "Free horizontal", en: "Free horizontal", zh: "自由摆放（横向）" },
    Entry { key: "Keep token statistics warm in the background (five minutes, 30-second scan limit)", en: "Keep token statistics warm in the background (five minutes, 30-second scan limit)", zh: "后台保留 Token 统计（5 分钟缓存，单次扫描最多 30 秒）" },
    Entry { key: "Tokens by local hour (recorded time buckets only)", en: "Tokens by local hour (recorded time buckets only)", zh: "按本机小时统计 Token（仅使用已记录的时间桶）" },
    Entry { key: "Native format not yet supported; accepts explicit local imports", en: "Native format not yet supported; accepts explicit local imports", zh: "暂不支持原生格式，可读取明确导出的本机记录" },
    Entry { key: "Some records are incomplete; totals cover only readable records.", en: "Some records are incomplete; totals cover only readable records.", zh: "部分记录不完整，合计只包含可读取的记录。" },
    Entry { key: "Usage by model (last 30 days)", en: "Usage by model (last 30 days)", zh: "按模型查看用量（最近 30 天）" },
    Entry { key: "Prompt cache sessions", en: "Prompt cache sessions", zh: "各会话提示缓存" },
    Entry { key: "No local records.", en: "No local records.", zh: "没有本机用量记录。" },
    Entry { key: "Refresh to read account details.", en: "Refresh to read account details.", zh: "点击刷新读取账户详情。" },
    Entry { key: "Counts may be incomplete.", en: "Counts may be incomplete.", zh: "计数可能不完整。" },
    Entry { key: "Untitled conversation", en: "Untitled conversation", zh: "未命名会话" },
    Entry { key: "{session} · {minutes} min remaining", en: "{session} · {minutes} min remaining", zh: "{session} · 至少还可复用 {minutes} 分钟" },
    Entry { key: "{model} · {share}% · {tokens} tokens · cache {hit}% · {speed} tokens/s · first token {first}s", en: "{model} · {share}% · {tokens} tokens · cache {hit}% · {speed} tokens/s · first token {first}s", zh: "{model} · 占比 {share}% · {tokens} tokens · 缓存命中 {hit}% · {speed} tokens/秒 · 首字 {first} 秒" },
    Entry { key: "Speed covers the last 24 hours, includes request wait, and needs three measured replies.", en: "Speed covers the last 24 hours, includes request wait, and needs three measured replies.", zh: "速度统计最近 24 小时，包含请求等待；至少有三次有效回复才显示。" },
    Entry { key: "Documented cache eligibility is a minimum; routing can still cause a cache miss.", en: "Documented cache eligibility is a minimum; routing can still cause a cache miss.", zh: "显示官方说明的最短缓存复用期；请求路由仍可能造成缓存未命中。" },
    Entry { key: "Cache eligible for at least {minutes} min; a hit is not guaranteed.", en: "Cache eligible for at least {minutes} min; a hit is not guaranteed.", zh: "缓存至少仍可复用 {minutes} 分钟，不保证请求命中。" },
    Entry { key: "Usage outside this computer was detected; value estimate paused.", en: "Usage outside this computer was detected; value estimate paused.", zh: "检测到这台电脑以外的用量，本周期暂停价值估算。" },
    Entry { key: "Storage and diagnostics", en: "Storage and diagnostics", zh: "存储与诊断" },
    Entry { key: "Statistics cache", en: "Statistics cache", zh: "统计缓存" },
    Entry { key: "Disk cache budget", en: "Disk cache budget", zh: "磁盘缓存预算" },
    Entry { key: "No disk cache", en: "No disk cache", zh: "禁用磁盘缓存" },
    Entry { key: "64 MiB (default)", en: "64 MiB (default)", zh: "64 MiB（默认）" },
    Entry { key: "Statistics cache: {size} MiB · {files} files", en: "Statistics cache: {size} MiB · {files} files", zh: "统计缓存：{size} MiB · {files} 个文件" },
    Entry { key: "Reading cache information…", en: "Reading cache information…", zh: "正在读取缓存信息…" },
    Entry { key: "Oldest statistics caches are removed when over budget. Oversized files are not saved. Account settings, keys and original logs are kept.", en: "Oldest statistics caches are removed when over budget. Oversized files are not saved. Account settings, keys and original logs are kept.", zh: "超出预算时优先移除旧统计缓存，单个文件超出预算时不保存。账户设置、密钥和原始日志会保留。" },
    Entry { key: "After clearing, statistics are rebuilt on the next read. The price table is retained.", en: "After clearing, statistics are rebuilt on the next read. The price table is retained.", zh: "清理后，下次读取时重新生成统计。价目表会保留。" },
    Entry { key: "Wait for the local scan to stop before clearing or changing the budget.", en: "Wait for the local scan to stop before clearing or changing the budget.", zh: "请等待本地扫描停止，再清理缓存或调整预算。" },
    Entry { key: "Refresh cache information", en: "Refresh cache information", zh: "刷新缓存信息" },
    Entry { key: "Clear statistics cache", en: "Clear statistics cache", zh: "清理统计缓存" },
    Entry { key: "Diagnostics", en: "Diagnostics", zh: "诊断信息" },
    Entry { key: "Export diagnostics", en: "Export diagnostics", zh: "导出诊断信息" },
    Entry { key: "Open diagnostics folder", en: "Open diagnostics folder", zh: "打开诊断文件夹" },
    Entry { key: "Export a local JSON report with version, process resources, cache sizes and scan status. Keys, account addresses and conversation contents are excluded. A new export replaces the previous report.", en: "Export a local JSON report with version, process resources, cache sizes and scan status. Keys, account addresses and conversation contents are excluded. A new export replaces the previous report.", zh: "导出包含版本、进程资源、缓存大小和扫描状态的本地 JSON 报告。报告不含密钥、账户地址或会话正文。再次导出会替换上一份报告。" },
    Entry { key: "Diagnostics exported locally.", en: "Diagnostics exported locally.", zh: "诊断信息已导出到本地。" },
    Entry { key: "Working…", en: "Working…", zh: "正在处理…" },
    Entry { key: "Removed {files} cache files · freed {size} MiB · {failed} failures", en: "Removed {files} cache files · freed {size} MiB · {failed} failures", zh: "已移除 {files} 个缓存文件 · 释放 {size} MiB · {failed} 个文件处理失败" },
    Entry { key: "Could not complete this operation. Check folder permissions and try again.", en: "Could not complete this operation. Check folder permissions and try again.", zh: "操作未能完成。请检查文件夹权限后重试。" },
    Entry { key: "Token spend", en: "Token spend", zh: "Token 消耗" },
    Entry { key: "Read local token spend", en: "Read local token spend", zh: "读取本地 Token 消耗" },
    Entry { key: "Local records · API value is an estimate, not your bill.", en: "Local records · API value is an estimate, not your bill.", zh: "本地记录 · API 价值为估算，并非实际账单。" },
    Entry { key: "Enable local reading to analyze records on this computer.", en: "Enable local reading to analyze records on this computer.", zh: "启用本地读取后，可分析这台电脑上的记录。" },
    Entry { key: "Reading local records…", en: "Reading local records…", zh: "正在读取本地记录…" },
    Entry { key: "Unable to read local records.", en: "Unable to read local records.", zh: "无法读取本地记录。" },
    Entry { key: "Only records available on this computer are included.", en: "Only records available on this computer are included.", zh: "仅统计这台电脑上可读取的记录。" },
    Entry { key: "All agents", en: "All agents", zh: "全部工具" },
    Entry { key: "All models", en: "All models", zh: "全部模型" },
    Entry { key: "Time range", en: "Time range", zh: "时间范围" },
    Entry { key: "Last 7 days", en: "Last 7 days", zh: "最近 7 天" },
    Entry { key: "Last 30 days", en: "Last 30 days", zh: "最近 30 天" },
    Entry { key: "Last 90 days", en: "Last 90 days", zh: "最近 90 天" },
    Entry { key: "All time", en: "All time", zh: "全部时间" },
    Entry { key: "Agent", en: "Agent", zh: "工具" },
    Entry { key: "Model", en: "Model", zh: "模型" },
    Entry { key: "Group by", en: "Group by", zh: "分组方式" },
    Entry { key: "Agents", en: "Agents", zh: "工具" },
    Entry { key: "Models", en: "Models", zh: "模型" },
    Entry { key: "Days", en: "Days", zh: "日期" },
    Entry { key: "Sort by", en: "Sort by", zh: "排序依据" },
    Entry { key: "Name / date", en: "Name / date", zh: "名称 / 日期" },
    Entry { key: "Total tokens", en: "Total tokens", zh: "Token 总量" },
    Entry { key: "API value", en: "API value", zh: "API 价值" },
    Entry { key: "Input", en: "Input", zh: "输入" },
    Entry { key: "Output", en: "Output", zh: "输出" },
    Entry { key: "Cache read", en: "Cache read", zh: "缓存读取" },
    Entry { key: "Cache write", en: "Cache write", zh: "缓存写入" },
    Entry { key: "Order", en: "Order", zh: "排序方向" },
    Entry { key: "Descending", en: "Descending", zh: "降序" },
    Entry { key: "Ascending", en: "Ascending", zh: "升序" },
    Entry { key: "Tokens without public prices:", en: "Tokens without public prices:", zh: "未公开价格的 Token：" },
    Entry { key: "Tokens without a kind breakdown:", en: "Tokens without a kind breakdown:", zh: "缺少类型明细的 Token：" },
    Entry { key: "Cache hit rate", en: "Cache hit rate", zh: "缓存命中率" },
    Entry { key: "No measured token records in this range.", en: "No measured token records in this range.", zh: "此范围内没有实测 Token 记录。" },
    Entry { key: "Unpriced", en: "Unpriced", zh: "未计价" },
    Entry { key: "Previous", en: "Previous", zh: "上一页" },
    Entry { key: "Next", en: "Next", zh: "下一页" },
    Entry { key: "* API value excludes tokens without a published price.", en: "* API value excludes tokens without a published price.", zh: "* API 价值不包含未公开价格的 Token。" },
    Entry { key: "Store not found", en: "Store not found", zh: "未找到记录目录" },
    Entry { key: "Native token records", en: "Native token records", zh: "工具原生 Token 记录" },
    Entry { key: "No token counters found", en: "No token counters found", zh: "未找到 Token 计数" },
    Entry { key: "Source coverage", en: "Source coverage", zh: "数据来源覆盖" },
    Entry { key: "Cancel scan", en: "Cancel scan", zh: "取消扫描" },
    Entry { key: "Stopping local scan…", en: "Stopping local scan…", zh: "正在停止本地扫描…" },
    Entry { key: "Local scan cancelled.", en: "Local scan cancelled.", zh: "已取消本地扫描。" },
    Entry { key: "Calculating token statistics…", en: "Calculating token statistics…", zh: "正在计算 Token 统计…" },
    Entry { key: "Unable to calculate token statistics.", en: "Unable to calculate token statistics.", zh: "无法计算 Token 统计。" },
    Entry { key: "Token statistics calculation cancelled.", en: "Token statistics calculation cancelled.", zh: "已取消 Token 统计计算。" },
    Entry { key: "Previous results remain displayed.", en: "Previous results remain displayed.", zh: "当前仍显示上次完成的结果。" },
    Entry { key: "Scanning {source} · sources {done}/{total} · files read {files}", en: "Scanning {source} · sources {done}/{total} · files read {files}", zh: "正在扫描 {source} · 来源 {done}/{total} · 已读取 {files} 个文件" },
    Entry { key: "Updates", en: "Updates", zh: "更新" },
    Entry { key: "Check for updates automatically", en: "Check for updates automatically", zh: "自动检查更新" },
    Entry { key: "Check for updates", en: "Check for updates", zh: "检查更新" },
    Entry { key: "Check once a day when enabled. Download and installation are your choice.", en: "Check once a day when enabled. Download and installation are your choice.", zh: "开启后每天检查一次。是否下载和安装由你选择。" },
    Entry { key: "Updates are installed only when you choose.", en: "Updates are installed only when you choose.", zh: "更新仅在你选择时安装。" },
    Entry { key: "Checking for updates…", en: "Checking for updates…", zh: "正在检查更新…" },
    Entry { key: "You have the latest Windows release.", en: "You have the latest Windows release.", zh: "当前已是最新 Windows 正式版。" },
    Entry { key: "Could not check for updates. Try again later or open the releases page.", en: "Could not check for updates. Try again later or open the releases page.", zh: "检查更新失败。请稍后重试，或打开发布页面。" },
    Entry { key: "Open releases page", en: "Open releases page", zh: "打开发布页面" },
    Entry { key: "Windows update available:", en: "Windows update available:", zh: "可用的 Windows 更新：" },
    Entry { key: " (reminders skipped)", en: " (reminders skipped)", zh: "（已跳过此版本提醒）" },
    Entry { key: "Open update page", en: "Open update page", zh: "打开更新页面" },
    Entry { key: "Skip this version", en: "Skip this version", zh: "跳过此版本" },
    Entry { key: "Open Settings → About to choose an update.", en: "Open Settings → About to choose an update.", zh: "在设置 → 关于中选择更新。" },
    Entry { key: "No QuotaScope servers, no QuotaScope account, no telemetry. Provider requests follow the Windows system proxy settings. Optional update checks contact GitHub.", en: "No QuotaScope servers, no QuotaScope account, no telemetry. Provider requests follow the Windows system proxy settings. Optional update checks contact GitHub.", zh: "无 QuotaScope 服务器，无 QuotaScope 账户，无遥测。供应商请求遵循 Windows 系统代理设置；可选更新检查会连接 GitHub。" },
    Entry { key: "Show Codex reset credits", en: "Show Codex reset credits", zh: "显示 Codex 重置次数" },
    Entry { key: "Reset credits: {count}", en: "Reset credits: {count}", zh: "可提前重置：{count} 次" },
    Entry { key: "Next expiry: {time}", en: "Next expiry: {time}", zh: "最早到期：{time}" },
    Entry { key: "Install Codex CLI to read reset credits.", en: "Install Codex CLI to read reset credits.", zh: "安装 Codex CLI 后可读取重置次数。" },
    Entry { key: "Reset credits: not available.", en: "Reset credits: not available.", zh: "暂无法读取重置次数。" },
    Entry { key: "Reading reset credits…", en: "Reading reset credits…", zh: "正在读取重置次数…" },
    Entry { key: "Account lifetime: {tokens} tokens", en: "Account lifetime: {tokens} tokens", zh: "账户累计：{tokens} token" },
    Entry { key: "Peak day: {tokens} tokens", en: "Peak day: {tokens} tokens", zh: "单日最高：{tokens} token" },
    Entry { key: "Current streak: {days} days", en: "Current streak: {days} days", zh: "当前连续使用：{days} 天" },
    Entry { key: "Longest streak: {days} days", en: "Longest streak: {days} days", zh: "最长连续使用：{days} 天" },
    Entry { key: "Cache hit rate: {percent}%", en: "Cache hit rate: {percent}%", zh: "缓存命中率：{percent}%" },
    Entry { key: "Prompt cache: {count} chats", en: "Prompt cache: {count} chats", zh: "提示词缓存：{count} 个会话" },
    Entry { key: "{tier} cache · expires in {minutes} min", en: "{tier} cache · expires in {minutes} min", zh: "{tier} 缓存 · {minutes} 分钟后到期" },
    Entry { key: "Prompt cache has expired.", en: "Prompt cache has expired.", zh: "提示词缓存已到期。" },
    Entry { key: "The next message will rebuild the cache.", en: "The next message will rebuild the cache.", zh: "下一条消息会重建缓存。" },
    Entry { key: "No active prompt cache.", en: "No active prompt cache.", zh: "暂无有效的提示词缓存。" },
    Entry { key: "No recent cache tier was reported.", en: "No recent cache tier was reported.", zh: "近期记录未报告缓存层级。" },
    Entry { key: "Reading prompt cache…", en: "Reading prompt cache…", zh: "正在读取提示词缓存…" },
    Entry { key: "Cache lifetime comes from local records.", en: "Cache lifetime comes from local records.", zh: "缓存有效期以本机记录为准。" },
    Entry { key: "Daily limit", en: "Daily limit", zh: "每日限额" },
    Entry { key: "Messages", en: "Messages", zh: "消息额度" },
    Entry { key: "Top-up allowance", en: "Top-up allowance", zh: "加购额度" },
    Entry { key: "Credits", en: "Credits", zh: "积分额度" },
    Entry { key: "Shared credits", en: "Shared credits", zh: "团队共享积分" },
    Entry { key: "This account has no plan with usage limits.", en: "This account has no plan with usage limits.", zh: "此账户没有提供用量额度的套餐。" },
    Entry { key: "{amount} credits expire {time}", en: "{amount} credits expire {time}", zh: "{amount} 积分于 {time} 到期" },
    Entry { key: "Configure", en: "Configure", zh: "配置" },
    Entry { key: "Collapse", en: "Collapse", zh: "收起" },
    Entry { key: "Disabled", en: "Disabled", zh: "未启用" },
    Entry { key: "Waiting for usage", en: "Waiting for usage", zh: "正在读取用量" },
    Entry { key: "{provider} Usage", en: "{provider} Usage", zh: "{provider} 用量" },
    Entry { key: "Enter a positive budget for My budget.", en: "Enter a positive budget for My budget.", zh: "“我的预算”需要一个正数预算金额。" },
    Entry { key: "No readable local history in the last 30 days.", en: "No readable local history in the last 30 days.", zh: "近 30 天没有可读取的本机用量记录。" },
    Entry { key: "Open usage page", en: "Open usage page", zh: "打开用量页" },
    Entry { key: "Search accounts", en: "Search accounts", zh: "搜索账户" },
    Entry { key: "Subscriptions", en: "Subscriptions", zh: "订阅账户" },
    Entry { key: "API accounts", en: "API accounts", zh: "API 账户" },
    Entry { key: "No matching accounts.", en: "No matching accounts.", zh: "没有匹配的账户。" },
    Entry { key: "Show credential", en: "Show credential", zh: "显示凭据明文" },
    Entry { key: "Detailed card", en: "Detailed card", zh: "详细用量卡" },
    Entry { key: "Ring measures", en: "Ring measures", zh: "用量环口径" },
    Entry { key: "Since top-up", en: "Since top-up", zh: "自充值以来" },
    Entry { key: "Balance only", en: "Balance only", zh: "仅显示余额" },
    Entry { key: "My budget", en: "My budget", zh: "我的预算" },
    Entry { key: "Budget in the balance's currency", en: "Budget in the balance's currency", zh: "预算金额（与余额币种一致）" },
    Entry { key: "Warn below", en: "Warn below", zh: "余额低于此数时提醒" },
    Entry { key: "Amount in the balance's currency; blank disables", en: "Amount in the balance's currency; blank disables", zh: "金额（与余额币种一致），留空关闭" },
    Entry { key: "Save balance settings", en: "Save balance settings", zh: "保存余额设置" },
    Entry { key: "Enter a positive amount, or leave blank to turn it off.", en: "Enter a positive amount, or leave blank to turn it off.", zh: "请输入正数金额，或留空关闭。" },
    Entry { key: "Applies when the provider reports a balance without its own limits. Low-balance warnings also require notifications to be enabled.", en: "Applies when the provider reports a balance without its own limits. Low-balance warnings also require notifications to be enabled.", zh: "余额口径适用于服务商只报告余额、未报告自身限额的读数。低余额提醒还需要启用通知。" },
    Entry { key: "Updated {time}", en: "Updated {time}", zh: "更新于{time}" },
    Entry { key: "{window}: {percent}% of window elapsed", en: "{window}: {percent}% of window elapsed", zh: "{window}：窗口时间已过 {percent}%" },
    Entry { key: "Recent usage (30 days)", en: "Recent usage (30 days)", zh: "近期用量（30 天）" },
    Entry { key: "Reading history…", en: "Reading history…", zh: "正在读取用量历史…" },
    Entry { key: "Add an API key to read history.", en: "Add an API key to read history.", zh: "添加 API key 后可读取用量历史。" },
    Entry { key: "Couldn't read the history.", en: "Couldn't read the history.", zh: "无法读取用量历史。" },
    Entry { key: "No history in the last 30 days.", en: "No history in the last 30 days.", zh: "近 30 天没有用量历史。" },
    Entry { key: "{tokens} tokens", en: "{tokens} tokens", zh: "{tokens} token" },
    Entry { key: "{tokens} tokens · API value ≈${cost}", en: "{tokens} tokens · API value ≈${cost}", zh: "{tokens} token · API 价值约 ${cost}" },
    Entry { key: "Most used: {model}", en: "Most used: {model}", zh: "使用最多：{model}" },
    Entry { key: "{tokens} tokens have no published price", en: "{tokens} tokens have no published price", zh: "{tokens} token 无公开价格" },
    Entry { key: "Provider statistics · all machines · no price breakdown", en: "Provider statistics · all machines · no price breakdown", zh: "服务商统计 · 所有设备 · 无价格明细" },
    Entry { key: "Local records · API value is an estimate, not a bill", en: "API value is an estimate, not a bill", zh: "API 价值估算，非实际账单" },
    Entry {
        key: "Antigravity omits unclassified token counts; records without a turn time use the conversation start time.",
        en: "Antigravity omits unclassified token counts; records without a turn time use the conversation start time.",
        zh: "Antigravity 会省略含义未确认的 token 计数；缺少调用时间的记录按会话开始时间归日。",
    },
    Entry { key: "On this PC", en: "On this PC", zh: "本机活动" },
    Entry { key: "Whole account", en: "Whole account", zh: "全账户活动" },
    Entry { key: "Today", en: "Today", zh: "今日" },
    Entry { key: "7 days", en: "7 days", zh: "7 天" },
    Entry { key: "30 days", en: "30 days", zh: "30 天" },
    Entry { key: "Time {percent}%", en: "Time {percent}%", zh: "时间 {percent}%" },
    Entry { key: "Gateway address", en: "Gateway address", zh: "网关地址" },
    Entry { key: "Enter a local or HTTPS gateway address.", en: "Enter a local or HTTPS gateway address.", zh: "请输入本机或 HTTPS 网关地址。" },
    Entry { key: "Add a gateway address in Settings.", en: "Add a gateway address in Settings.", zh: "请在设置中添加网关地址。" },
    Entry { key: "That gateway address is not allowed.", en: "That gateway address is not allowed.", zh: "此网关地址不被允许。" },
    Entry { key: "Show panel", en: "Show panel", zh: "显示面板" },
    Entry { key: "Refresh all", en: "Refresh all", zh: "全部刷新" },
    Entry { key: "Settings", en: "Settings", zh: "设置" },
    Entry { key: "Exit", en: "Exit", zh: "退出" },
    Entry { key: "General", en: "General", zh: "常规" },
    Entry { key: "Accounts", en: "Accounts", zh: "账户" },
    Entry { key: "Notifications", en: "Notifications", zh: "通知" },
    Entry { key: "About", en: "About", zh: "关于" },
    Entry { key: "Panel", en: "Panel", zh: "面板" },
    Entry { key: "Dock to", en: "Dock to", zh: "停靠位置" },
    Entry { key: "Right", en: "Right", zh: "右侧" },
    Entry { key: "Left", en: "Left", zh: "左侧" },
    Entry { key: "Top", en: "Top", zh: "顶部" },
    Entry { key: "Floating", en: "Floating", zh: "悬浮" },
    Entry { key: "Panel size", en: "Panel size", zh: "面板大小" },
    Entry { key: "Small", en: "Small", zh: "小" },
    Entry { key: "Standard", en: "Standard", zh: "标准" },
    Entry { key: "Large", en: "Large", zh: "大" },
    Entry { key: "Ring spacing", en: "Ring spacing", zh: "环间距" },
    Entry { key: "Tight", en: "Tight", zh: "紧凑" },
    Entry { key: "Loose", en: "Loose", zh: "宽松" },
    Entry { key: "Show percent labels", en: "Show percent labels", zh: "显示百分比标签" },
    Entry { key: "Show what's left", en: "Show what's left", zh: "显示剩余量" },
    Entry { key: "Show window clock", en: "Show window clock", zh: "显示窗口时钟" },
    Entry { key: "Time gone", en: "Time gone", zh: "已过时间" },
    Entry { key: "Time left", en: "Time left", zh: "剩余时间" },
    Entry {
        key: "Clock shows",
        en: "Clock shows",
        zh: "时钟显示",
    },
    Entry { key: "Turn red past", en: "Turn red past", zh: "超过此数变红" },
    Entry {
        key: "Estimated value {value} · {spent} used",
        en: "Estimated value {value} · {spent} used",
        zh: "价值推算 {value} · 已用 {spent}",
    },
    Entry {
        key: "Read token spend",
        en: "Read token spend",
        zh: "读取 token 花费",
    },
    Entry {
        key: "Read this machine's Claude Code and Codex transcripts and Antigravity conversation databases, then price known token counts at published API rates. These records never leave this machine.",
        en: "Read this machine's Claude Code and Codex transcripts and Antigravity conversation databases, then price known token counts at published API rates. These records never leave this machine.",
        zh: "读取本机的 Claude Code、Codex 会话记录和 Antigravity 会话数据库，按公开 API 价格估算已知 token 的价值。记录不会离开这台电脑。",
    },
    Entry {
        key: "Hide the tray icon",
        en: "Hide the tray icon",
        zh: "隐藏托盘图标",
    },
    Entry {
        key: "The panel stays put and comes back at every launch; run QuotaScope again to reach Settings.",
        en: "The panel stays put and comes back at every launch; run QuotaScope again to reach Settings.",
        zh: "面板保留，且每次启动都会回来；再次运行 QuotaScope 即可打开设置。",
    },
    Entry { key: "Show second ring", en: "Show second ring", zh: "显示第二道环" },
    Entry { key: "Show forecast", en: "Show forecast", zh: "显示预测" },
    Entry { key: "Auto-collapse when idle", en: "Auto-collapse when idle", zh: "空闲时自动收起" },
    Entry { key: "Follow the active display", en: "Follow the active display", zh: "跟随活动显示器" },
    Entry { key: "Refresh", en: "Refresh", zh: "刷新" },
    Entry { key: "Refresh interval", en: "Refresh interval", zh: "刷新间隔" },
    Entry { key: "Automatic", en: "Automatic", zh: "自动" },
    Entry { key: "30s", en: "30s", zh: "30秒" },
    Entry { key: "1min", en: "1min", zh: "1分钟" },
    Entry { key: "2min", en: "2min", zh: "2分钟" },
    Entry { key: "5min", en: "5min", zh: "5分钟" },
    Entry { key: "10min", en: "10min", zh: "10分钟" },
    Entry { key: "30min", en: "30min", zh: "30分钟" },
    Entry { key: "Windows", en: "Windows", zh: "Windows" },
    Entry { key: "Launch at startup", en: "Launch at startup", zh: "开机时启动" },
    Entry {
        key: "Each provider reports its own figures by the route that product offers — QuotaScope holds no account of its own and sends nothing anywhere but to the provider you already use.",
        en: "Each provider reports its own figures by the route that product offers — QuotaScope holds no account of its own and sends nothing anywhere but to the provider you already use.",
        zh: "每个服务商都通过自家产品提供的接口上报数据——QuotaScope 不持有自己的账户，除了你已在使用的服务商之外不向任何地方发送请求。",
    },
    Entry {
        key: "Reads the login this tool already saved on this PC.",
        en: "Reads the login this tool already saved on this PC.",
        zh: "读取该工具已保存在此电脑上的登录凭据。",
    },
    Entry { key: "Sign out", en: "Sign out", zh: "退出登录" },
    Entry { key: "Sign in with GitHub", en: "Sign in with GitHub", zh: "使用 GitHub 登录" },
    Entry {
        key: "Device-code sign-in. Requests read:user only — never your repositories. The consent page names the editor whose client it borrows; this is not an official integration.",
        en: "Device-code sign-in. Requests read:user only — never your repositories. The consent page names the editor whose client it borrows; this is not an official integration.",
        zh: "设备码登录。仅请求 read:user 权限——绝不触及你的仓库。授权页面显示的是它所借用编辑器的客户端名称；这并非官方集成。",
    },
    Entry { key: "API key", en: "API key", zh: "API 密钥" },
    Entry { key: "Paste the API key", en: "Paste the API key", zh: "粘贴 API 密钥" },
    Entry { key: "Save", en: "Save", zh: "保存" },
    Entry { key: "Not available on Windows.", en: "Not available on Windows.", zh: "在 Windows 上暂不可用。" },
    Entry {
        key: "Reads the language server Antigravity runs while it is open — figures exist only while it is running.",
        en: "Reads the language server Antigravity runs while it is open — figures exist only while it is running.",
        zh: "读取 Antigravity 打开时运行的语言服务器——仅在其运行期间可读。",
    },
    Entry {
        key: "Reads the login Cursor already saved in its own database.",
        en: "Reads the login Cursor already saved in its own database.",
        zh: "读取 Cursor 存在自己数据库里的登录凭据。",
    },
    Entry {
        key: "Open Antigravity to see its usage.",
        en: "Open Antigravity to see its usage.",
        zh: "打开 Antigravity 才能读到它的用量。",
    },
    Entry {
        key: "Antigravity is open but didn't answer. Restarting it usually helps.",
        en: "Antigravity is open but didn't answer. Restarting it usually helps.",
        zh: "Antigravity 开着但没有响应，重启它通常能解决。",
    },
    Entry {
        key: "Sign in to Cursor to see usage.",
        en: "Sign in to Cursor to see usage.",
        zh: "在 Cursor 中登录后才能读到用量。",
    },
    Entry {
        key: "Cursor's saved login was refused. Open Cursor to renew it.",
        en: "Cursor's saved login was refused. Open Cursor to renew it.",
        zh: "Cursor 保存的登录被拒绝了。打开 Cursor 让它重新登录。",
    },
    Entry {
        key: "Reads a browser session cookie — browser access has not been ported yet.",
        en: "Reads a browser session cookie — browser access has not been ported yet.",
        zh: "读取浏览器会话 Cookie——浏览器访问尚未移植。",
    },
    Entry {
        key: "Reads the login Cursor saved. Enable Cursor's Grok Bot through Cursor until this route is ported.",
        en: "Reads the login Cursor saved. Enable Cursor's Grok Bot through Cursor until this route is ported.",
        zh: "读取 Cursor 保存的登录凭据。在此路线移植前，请通过 Cursor 启用其 Grok Bot。",
    },
    Entry {
        key: "Signs Volcengine's usage API with access keys — the signer has not been ported yet.",
        en: "Signs Volcengine's usage API with access keys — the signer has not been ported yet.",
        zh: "使用访问密钥对火山引擎的用量 API 签名——签名器尚未移植。",
    },
    Entry { key: "Warn me when a limit passes", en: "Warn me when a limit passes", zh: "当限额超过以下值时提醒我" },
    Entry {
        key: "Uses kiro-cli's existing login. Sign in with kiro-cli login first, then enable Kiro and refresh. No credential needs to be pasted.",
        en: "Uses kiro-cli's existing login. Sign in with kiro-cli login first, then enable Kiro and refresh. No credential needs to be pasted.",
        zh: "使用 kiro-cli 已保存的登录。先运行 kiro-cli login，然后启用 Kiro 并刷新，无需粘贴凭据。",
    },
    Entry {
        key: "Reads Devin's local or API quota — the Windows route has not been ported yet.",
        en: "Reads Devin's local or API quota — the Windows route has not been ported yet.",
        zh: "读取 Devin 的本机或 API 配额——Windows 路线尚未移植。",
    },
    Entry {
        key: "Enter the gateway address and API key to enable this account.",
        en: "Enter the gateway address and API key to enable this account.",
        zh: "输入网关地址和 API 密钥以启用此账户。",
    },
    Entry {
        key: "Reads browser local storage — that Windows route has not been ported yet.",
        en: "Reads browser local storage — that Windows route has not been ported yet.",
        zh: "读取浏览器本地存储——Windows 路线尚未移植。",
    },
    Entry {
        key: "This provider is catalogued, but its Windows usage route has not been ported yet.",
        en: "This provider is catalogued, but its Windows usage route has not been ported yet.",
        zh: "此服务商已列入目录，但 Windows 用量读取路线尚未移植。",
    },
    Entry { key: "when a warned window comes back", en: "when a warned window comes back", zh: "当被警告的窗口恢复时" },
    Entry { key: "when checks keep failing", en: "when checks keep failing", zh: "当检查持续失败时" },
    Entry {
        key: "All notifications are off until you turn them on.",
        en: "All notifications are off until you turn them on.",
        zh: "在你开启之前，所有通知都保持关闭。",
    },
    Entry { key: "Enable notifications", en: "Enable notifications", zh: "启用通知" },
    Entry {
        key: "A screen-edge monitor for your AI coding allowances.",
        en: "A screen-edge monitor for your AI coding allowances.",
        zh: "屏幕边缘的 AI 编码额度监视器。",
    },
    Entry { key: "Version", en: "Version", zh: "版本" },
    Entry {
        key: "No QuotaScope servers, no QuotaScope account, no telemetry. Requests go to the providers you already use and follow the Windows system proxy settings.",
        en: "No QuotaScope servers, no QuotaScope account, no telemetry. Requests go to the providers you already use and follow the Windows system proxy settings.",
        zh: "没有 QuotaScope 服务器、没有 QuotaScope 账户、没有遥测。请求直达你已在使用的服务商，并遵循 Windows 系统代理设置。",
    },
    Entry {
        key: "Loading…",
        en: "Loading…",
        zh: "读取中…",
    },
    Entry {
        key: "notConnected",
        en: "Connect Claude Code in Settings to see usage.",
        zh: "在设置中连接 Claude Code 以查看用量。",
    },
    Entry {
        key: "awaitingResponse",
        en: "Waiting for the next Claude Code response.",
        zh: "等待 Claude Code 的下一次响应。",
    },
    Entry {
        key: "No limits reported.",
        en: "No limits reported.",
        zh: "没有报告任何限额。",
    },
    Entry {
        key: "signInRequired",
        en: "Sign in to Codex to see usage.",
        zh: "登录 Codex 以查看用量。",
    },
    Entry {
        key: "claudeSignInRequired",
        en: "Sign in to Claude Code to see usage.",
        zh: "登录 Claude Code 以查看用量。",
    },
    Entry {
        key: "claudeLoginExpired",
        en: "Claude Code's saved login expired. Use Claude Code, or connect the status line.",
        zh: "Claude Code 保存的登录已过期。使用一次 Claude Code，或连接状态栏。",
    },
    Entry {
        key: "Codex isn't installed.",
        en: "Codex isn't installed.",
        zh: "尚未安装 Codex。",
    },
    Entry {
        key: "codexServerFailed",
        en: "Couldn't start the Codex helper.",
        zh: "无法启动 Codex 助手。",
    },
    Entry {
        key: "Kiro CLI isn't installed.",
        en: "Kiro CLI isn't installed.",
        zh: "尚未安装 Kiro CLI，或 kiro-cli.exe 不在 PATH 中。",
    },
    Entry {
        key: "Sign in to Kiro CLI to see usage.",
        en: "Sign in to Kiro CLI to see usage.",
        zh: "请先运行 kiro-cli login 登录，以查看用量。",
    },
    Entry {
        key: "Update Kiro CLI to read subscription usage.",
        en: "Update Kiro CLI to read subscription usage.",
        zh: "请更新 Kiro CLI，以读取订阅用量。",
    },
    Entry {
        key: "grokSignInRequired",
        en: "Sign in to Grok to see usage.",
        zh: "登录 Grok 以查看用量。",
    },
    Entry {
        key: "grokLoginExpired",
        en: "Grok's saved login expired. Use Grok to renew it.",
        zh: "Grok 保存的登录已过期。使用一次 Grok 以续期。",
    },
    Entry {
        key: "signedOut",
        en: "Sign in to this account again in Settings.",
        zh: "请在设置中重新登录此账号。",
    },
    Entry {
        key: "notSignedIn",
        en: "Sign in from Settings to see usage.",
        zh: "请在设置中登录以查看用量。",
    },
    Entry {
        key: "Import a browser session in Settings.",
        en: "Import a browser session in Settings.",
        zh: "请在设置中导入浏览器会话。",
    },
    Entry {
        key: "Import from browser",
        en: "Import from browser",
        zh: "从浏览器导入",
    },
    Entry {
        key: "Imported the session from {browser}.",
        en: "Imported the session from {browser}.",
        zh: "已从 {browser} 导入会话。",
    },
    Entry {
        key: "No matching session found in your browsers.",
        en: "No matching session found in your browsers.",
        zh: "在浏览器里没有找到匹配的会话。",
    },
    Entry {
        key: "Uses a browser session. Importing reads the site's cookies from your browsers — the one you open links with first. A pasted Cookie header works too.",
        en: "Uses a browser session. Importing reads the site's cookies from your browsers — the one you open links with first. A pasted Cookie header works too.",
        zh: "使用浏览器会话。导入会从你的浏览器读取该站点的 Cookie——优先读取你设为默认的浏览器。也可以直接粘贴 Cookie 头。",
    },
    Entry {
        key: "Or paste a Cookie header",
        en: "Or paste a Cookie header",
        zh: "或粘贴 Cookie 头",
    },
    Entry {
        key: "Paste the Cookie header",
        en: "Paste the Cookie header",
        zh: "粘贴 Cookie 头",
    },
    Entry {
        key: "The browser session has expired — import it again in Settings.",
        en: "The browser session has expired — import it again in Settings.",
        zh: "浏览器会话已过期——请在设置中重新导入。",
    },
    Entry {
        key: "Add an API key in Settings.",
        en: "Add an API key in Settings.",
        zh: "请在设置中添加 API 密钥。",
    },
    Entry {
        key: "That key was refused. Check it in Settings.",
        en: "That key was refused. Check it in Settings.",
        zh: "该密钥被拒绝，请在设置中检查。",
    },
    Entry {
        key: "The service didn't respond.",
        en: "The service didn't respond.",
        zh: "服务没有响应。",
    },
    Entry {
        key: "Couldn't read the reply.",
        en: "Couldn't read the reply.",
        zh: "无法读取响应。",
    },
    Entry {
        key: "Checking too often — easing off.",
        en: "Checking too often — easing off.",
        zh: "检查过于频繁，已自动放缓。",
    },
    Entry {
        key: "The service returned an error.",
        en: "The service returned an error.",
        zh: "服务返回了错误。",
    },
    Entry {
        key: "notOnWindows",
        en: "This provider's route hasn't been ported to Windows yet.",
        zh: "该服务商的数据路由尚未移植到 Windows。",
    },
    Entry {
        key: "zaiNoCodingPlan",
        en: "That key works. The account has no Coding Plan running on it.",
        zh: "密钥有效，但该账号没有正在生效的编码套餐。",
    },
    Entry {
        key: "extensionMissing",
        en: "The extension's program isn't there, or can't be run. Check its folder.",
        zh: "扩展的程序不存在或无法运行。请检查其文件夹。",
    },
    Entry {
        key: "extensionTimedOut",
        en: "The extension ran past its own time limit.",
        zh: "扩展运行超出了自身时限。",
    },
    Entry {
        key: "extensionFailed",
        en: "The extension exited without a readable report.",
        zh: "扩展已退出，但未输出可读的报告。",
    },
    Entry {
        key: "extensionSignedOut",
        en: "The extension says its account is signed out.",
        zh: "扩展报告其账号已退出登录。",
    },
    Entry {
        key: "No quotascope-extension.json in this folder.",
        en: "No quotascope-extension.json in this folder.",
        zh: "该文件夹中没有 quotascope-extension.json。",
    },
    Entry {
        key: "quotascope-extension.json isn't valid JSON, or is missing a field.",
        en: "quotascope-extension.json isn't valid JSON, or is missing a field.",
        zh: "quotascope-extension.json 不是有效的 JSON，或缺少字段。",
    },
    Entry {
        key: "Written for extension schema {n}, which this version of QuotaScope doesn't read.",
        en: "Written for extension schema {n}, which this version of QuotaScope doesn't read.",
        zh: "该清单使用的 schema 版本为 {n}，此版本的 QuotaScope 无法读取。",
    },
    Entry {
        key: "Its id must be lowercase letters, digits, dots, dashes or underscores, 64 at most.",
        en: "Its id must be lowercase letters, digits, dots, dashes or underscores, 64 at most.",
        zh: "id 只能包含小写字母、数字、点、横线或下划线，最长 64 个字符。",
    },
    Entry {
        key: "Its name is empty.",
        en: "Its name is empty.",
        zh: "其名称为空。",
    },
    Entry {
        key: "Its program has to be inside its own folder.",
        en: "Its program has to be inside its own folder.",
        zh: "其程序必须位于自己的文件夹内。",
    },
    Entry {
        key: "Its program isn't there, or can't be run.",
        en: "Its program isn't there, or can't be run.",
        zh: "其程序不存在，或无法运行。",
    },
    Entry {
        key: "Another extension already uses this id.",
        en: "Another extension already uses this id.",
        zh: "另一个扩展已使用了此 id。",
    },
    Entry { key: "Extensions", en: "Extensions", zh: "扩展" },
    Entry {
        key: "A program in the extensions folder reports one account's usage. It runs only while it is switched on here, and QuotaScope hands it no credential.",
        en: "A program in the extensions folder reports one account's usage. It runs only while it is switched on here, and QuotaScope hands it no credential.",
        zh: "扩展文件夹中的一个程序可上报一个账户的用量。只有在此处开启后才会运行，且 QuotaScope 不会向它提供任何凭据。",
    },
    Entry {
        key: "No extensions found.",
        en: "No extensions found.",
        zh: "未发现扩展。",
    },
    Entry { key: "Look again", en: "Look again", zh: "重新扫描" },
    Entry {
        key: "5-hour limit",
        en: "5-hour limit",
        zh: "5 小时限额",
    },
    Entry {
        key: "Weekly limit",
        en: "Weekly limit",
        zh: "每周限额",
    },
    Entry {
        key: "Spend limit",
        en: "Spend limit",
        zh: "消费限额",
    },
    Entry {
        key: "Balance",
        en: "Balance",
        zh: "余额",
    },
    Entry {
        key: "Monthly limit",
        en: "Monthly limit",
        zh: "每月限额",
    },
    Entry {
        key: "{n}-day limit",
        en: "{n}-day limit",
        zh: "{n} 天限额",
    },
    Entry {
        key: "{n}-hour limit",
        en: "{n}-hour limit",
        zh: "{n} 小时限额",
    },
    Entry {
        key: "estimated",
        en: "estimated",
        zh: "估算",
    },
    Entry {
        key: "since top-up",
        en: "since top-up",
        zh: "按充值额",
    },
    Entry {
        key: "of your budget",
        en: "of your budget",
        zh: "按你的预算",
    },
    Entry {
        key: "{n} days",
        en: "{n} days",
        zh: "{n} 天",
    },
    Entry {
        key: "{n} hours",
        en: "{n} hours",
        zh: "{n} 小时",
    },
    Entry {
        key: "under 15 minutes",
        en: "under 15 minutes",
        zh: "不到 15 分钟",
    },
    Entry {
        key: "about an hour",
        en: "about an hour",
        zh: "约 1 小时",
    },
    Entry {
        key: "about {n} minutes",
        en: "about {n} minutes",
        zh: "约 {n} 分钟",
    },
    Entry {
        key: "about {n} hours",
        en: "about {n} hours",
        zh: "约 {n} 小时",
    },
    Entry {
        key: "Credit balance",
        en: "Credit balance",
        zh: "余额",
    },
    Entry {
        key: "Resets {time}",
        en: "Resets {time}",
        zh: "{time} 重置",
    },
    Entry {
        key: "expires {time}",
        en: "expires {time}",
        zh: "{time} 到期",
    },
    Entry {
        key: "As of {time}",
        en: "As of {time}",
        zh: "截至 {time}",
    },
    Entry {
        key: "Reading may be out of date",
        en: "Reading may be out of date",
        zh: "读数可能已过期",
    },
    Entry {
        key: "{p} Used",
        en: "{p} Used",
        zh: "已用 {p}",
    },
    Entry {
        key: "{p} Left",
        en: "{p} Left",
        zh: "剩余 {p}",
    },
    Entry {
        key: "Runs out in {t}",
        en: "Runs out in {t}",
        zh: "预计 {t} 用尽",
    },
    Entry {
        key: "Won't last the window",
        en: "Won't last the window",
        zh: "预计在重置前用尽",
    },
    Entry {
        key: "Expected to last the window",
        en: "Expected to last the window",
        zh: "预计够用至重置",
    },
    Entry {
        key: "{n} points",
        en: "{n} points",
        zh: "{n} 积分",
    },
    Entry {
        key: "No reading",
        en: "No reading",
        zh: "暂无读数",
    },
    Entry {
        key: "Refreshing…",
        en: "Refreshing…",
        zh: "正在刷新…",
    },
    Entry {
        key: "{t} left, {w}",
        en: "{t} left, {w}",
        zh: "剩 {t}，{w}",
    },
    Entry {
        key: "{t} used, {w}",
        en: "{t} used, {w}",
        zh: "已用 {t}，{w}",
    },
    // Settings and shell strings.
    Entry {
        key: "Settings",
        en: "Settings",
        zh: "设置",
    },
    Entry {
        key: "General",
        en: "General",
        zh: "常规",
    },
    Entry {
        key: "Accounts",
        en: "Accounts",
        zh: "账号",
    },
    Entry {
        key: "Notifications",
        en: "Notifications",
        zh: "通知",
    },
    Entry {
        key: "About",
        en: "About",
        zh: "关于",
    },
    Entry {
        key: "Launch at startup",
        en: "Launch at startup",
        zh: "开机自动启动",
    },
    Entry {
        key: "Auto-collapse when idle",
        en: "Auto-collapse when idle",
        zh: "空闲时自动收起",
    },
    Entry {
        key: "Show percent labels",
        en: "Show percent labels",
        zh: "显示百分比标签",
    },
    Entry {
        key: "Show what's left",
        en: "Show what's left",
        zh: "显示剩余量",
    },
    Entry {
        key: "Show window clock",
        en: "Show window clock",
        zh: "显示窗口时间弧",
    },
    Entry {
        key: "Show second ring",
        en: "Show second ring",
        zh: "显示第二圆环",
    },
    Entry {
        key: "Show forecast",
        en: "Show forecast",
        zh: "显示用量预测",
    },
    Entry {
        key: "Follow the active display",
        en: "Follow the active display",
        zh: "跟随活动显示器",
    },
    Entry {
        key: "Panel size",
        en: "Panel size",
        zh: "面板大小",
    },
    Entry {
        key: "Small",
        en: "Small",
        zh: "小",
    },
    Entry {
        key: "Standard",
        en: "Standard",
        zh: "标准",
    },
    Entry {
        key: "Large",
        en: "Large",
        zh: "大",
    },
    Entry {
        key: "Ring spacing",
        en: "Ring spacing",
        zh: "圆环间距",
    },
    Entry {
        key: "Tight",
        en: "Tight",
        zh: "紧凑",
    },
    Entry {
        key: "Loose",
        en: "Loose",
        zh: "宽松",
    },
    Entry {
        key: "Refresh interval",
        en: "Refresh interval",
        zh: "刷新间隔",
    },
    Entry {
        key: "Automatic",
        en: "Automatic",
        zh: "自动",
    },
    Entry {
        key: "{n} seconds",
        en: "{n} seconds",
        zh: "{n} 秒",
    },
    Entry {
        key: "{n} minutes",
        en: "{n} minutes",
        zh: "{n} 分钟",
    },
    Entry {
        key: "1 minute",
        en: "1 minute",
        zh: "1 分钟",
    },
    Entry {
        key: "Warn me when a limit passes",
        en: "Warn me when a limit passes",
        zh: "当限额超过时提醒我",
    },
    Entry {
        key: "when a limit is spent",
        en: "when a limit is spent",
        zh: "当限额耗尽时",
    },
    Entry {
        key: "when a warned window comes back",
        en: "when a warned window comes back",
        zh: "被提醒过的窗口恢复时",
    },
    Entry {
        key: "when checks keep failing",
        en: "when checks keep failing",
        zh: "当检查连续失败时",
    },
    Entry {
        key: "when a balance falls under",
        en: "when a balance falls under",
        zh: "当余额低于",
    },
    Entry {
        key: "All notifications are off until you turn them on.",
        en: "All notifications are off until you turn them on.",
        zh: "所有通知默认关闭，需要你手动开启。",
    },
    Entry {
        key: "API key",
        en: "API key",
        zh: "API 密钥",
    },
    Entry {
        key: "Save",
        en: "Save",
        zh: "保存",
    },
    Entry {
        key: "Refresh now",
        en: "Refresh now",
        zh: "立即刷新",
    },
    Entry {
        key: "Copied",
        en: "Copied",
        zh: "已复制",
    },
    Entry {
        key: "Copy",
        en: "Copy",
        zh: "复制",
    },
    Entry {
        key: "Retry",
        en: "Retry",
        zh: "重试",
    },
    Entry {
        key: "Copy JSON report",
        en: "Copy JSON report",
        zh: "复制 JSON 报告",
    },
    Entry {
        key: "Open settings",
        en: "Open settings",
        zh: "打开设置",
    },
    Entry {
        key: "Refresh all",
        en: "Refresh all",
        zh: "全部刷新",
    },
    Entry {
        key: "Show panel",
        en: "Show panel",
        zh: "显示面板",
    },
    Entry {
        key: "Exit",
        en: "Exit",
        zh: "退出",
    },
    Entry {
        key: "Language",
        en: "Language",
        zh: "语言",
    },
    Entry {
        key: "English",
        en: "English",
        zh: "English",
    },
    Entry {
        key: "简体中文",
        en: "简体中文",
        zh: "简体中文",
    },
    Entry {
        key: "A limit passed {p}",
        en: "A limit passed {p}",
        zh: "某个限额已超过 {p}",
    },
    Entry {
        key: "A limit is spent",
        en: "A limit is spent",
        zh: "某个限额已耗尽",
    },
    Entry {
        key: "A limit is close",
        en: "A limit is close",
        zh: "某个限额即将用尽",
    },
    Entry {
        key: "{w} passed {p}",
        en: "{w} passed {p}",
        zh: "{w} 已超过 {p}",
    },
    Entry {
        key: "A warned window came back",
        en: "A warned window came back",
        zh: "被提醒过的窗口已恢复",
    },
    Entry {
        key: "Checks keep failing",
        en: "Checks keep failing",
        zh: "检查连续失败",
    },
    Entry {
        key: "Low balance",
        en: "Low balance",
        zh: "余额偏低",
    },
    Entry {
        key: "Not on Windows yet",
        en: "Not on Windows yet",
        zh: "尚未支持 Windows",
    },
    Entry {
        key: "Enabled",
        en: "Enabled",
        zh: "已启用",
    },
    Entry {
        key: "Route",
        en: "Route",
        zh: "数据来源",
    },
    Entry {
        key: "Usage endpoint",
        en: "Usage endpoint",
        zh: "用量接口",
    },
    Entry {
        key: "Reads the login this tool already saved on this PC.",
        en: "Reads the login this tool already saved on this PC.",
        zh: "读取该工具已在本机保存的登录。",
    },
    Entry {
        key: "Pin the limit the ring shows",
        en: "Pin the limit the ring shows",
        zh: "固定圆环显示的限额",
    },
    Entry {
        key: "Ring colour",
        en: "Ring colour",
        zh: "圆环颜色",
    },
    Entry {
        key: "By usage",
        en: "By usage",
        zh: "按用量",
    },
    Entry {
        key: "Version",
        en: "Version",
        zh: "版本",
    },
    Entry {
        key: "A screen-edge monitor for your AI coding allowances.",
        en: "A screen-edge monitor for your AI coding allowances.",
        zh: "停靠在屏幕边缘的 AI 编码额度监视器。",
    },
    Entry {
        key: "Diagnostics",
        en: "Diagnostics",
        zh: "诊断",
    },
    Entry {
        key: "Copy diagnostic report",
        en: "Copy diagnostic report",
        zh: "复制诊断报告",
    },
    Entry {
        key: "Last checked {time}",
        en: "Last checked {time}",
        zh: "上次检查 {time}",
    },
    Entry {
        key: "never",
        en: "never",
        zh: "从未",
    },
    Entry {
        key: "just now",
        en: "just now",
        zh: "刚刚",
    },
    Entry {
        key: "{n}m ago",
        en: "{n}m ago",
        zh: "{n} 分钟前",
    },
    Entry {
        key: "{n}h ago",
        en: "{n}h ago",
        zh: "{n} 小时前",
    },
    Entry {
        key: "{n}d ago",
        en: "{n}d ago",
        zh: "{n} 天前",
    },
];

/// The localized string for a key. Unmatched keys return the key itself, the
/// same way a missing translation falls through to English.
pub fn t(key: &str) -> &'static str {
    use std::sync::OnceLock;
    type Strings = std::collections::HashMap<String, String>;
    static TRADITIONAL: OnceLock<Strings> = OnceLock::new();
    static JAPANESE: OnceLock<Strings> = OnceLock::new();
    static KOREAN: OnceLock<Strings> = OnceLock::new();
    let translations = match current() {
        Language::TraditionalChinese => Some(TRADITIONAL.get_or_init(|| {
            serde_json::from_str(include_str!("translations/zh-Hant.json")).unwrap_or_default()
        })),
        Language::Japanese => Some(JAPANESE.get_or_init(|| {
            serde_json::from_str(include_str!("translations/ja.json")).unwrap_or_default()
        })),
        Language::Korean => Some(KOREAN.get_or_init(|| {
            serde_json::from_str(include_str!("translations/ko.json")).unwrap_or_default()
        })),
        _ => None,
    };
    if let Some(value) = translations.and_then(|table| table.get(key)) {
        return value.as_str();
    }
    let zh = current() == Language::Chinese;
    for entry in TABLE {
        if entry.key == key {
            return if zh { entry.zh } else { entry.en };
        }
    }
    // A miss is a bug in this file, but falling through to the key keeps
    // new call sites visible instead of blank. The keys are code literals,
    // so the leak is a finite, tiny set.
    Box::leak(key.to_string().into_boxed_str())
}

/// Interpolates `{xxx}`-style placeholders. Values are passed as strings so
/// an integer can never silently produce the wrong formatter.
pub fn t_fmt(key: &str, values: &[&str]) -> String {
    let mut out = t(key).to_string();
    for value in values {
        // Placeholders are positional, whatever they are named: the first
        // unfilled `{...}` gets the next value. (Matching only `{n}`
        // literally once left `{p} Used 7%` on screen.)
        let start = match out.find('{') {
            Some(s) => s,
            None => {
                out.push(' ');
                out.push_str(value);
                continue;
            }
        };
        let end = match out[start..].find('}') {
            Some(e) => start + e + 1,
            None => {
                out.push(' ');
                out.push_str(value);
                continue;
            }
        };
        out = format!("{}{}{}", &out[..start], value, &out[end..]);
    }
    out
}

pub fn provider_raw(provider: crate::model::Provider) -> &'static str {
    use crate::model::Provider::*;
    match provider {
        ClaudeCode => "claudeCode",
        Codex => "codex",
        Kiro => "kiro",
        Antigravity => "antigravity",
        Cursor => "cursor",
        OpenCodeGo => "openCodeGo",
        KimiCode => "kimiCode",
        OllamaCloud => "ollamaCloud",
        Zai => "zai",
        GlmCoding => "glmCoding",
        Minimax => "minimax",
        MinimaxCn => "minimaxCN",
        Copilot => "copilot",
        Grok => "grok",
        GrokBot => "grokBot",
        Volcengine => "volcengine",
        CommandCode => "commandCode",
        DeepSeek => "deepSeek",
        Devin => "devin",
        XiaomiMiMo => "xiaomiMiMo",
        Sub2api => "sub2api",
        NewApi => "newAPI",
        V2ex => "v2ex",
        Qoder => "qoder",
        StepFun => "stepFun",
        ClinePass => "clinePass",
        AlibabaCodingPlan => "alibabaCodingPlan",
        AlibabaTokenPlan => "alibabaTokenPlan",
        QwenCloud => "qwenCloud",
        Factory => "factory",
        Gemini => "gemini",
        KiloCode => "kiloCode",
        Augment => "augment",
        JetBrainsAi => "jetBrainsAI",
        T3Chat => "t3Chat",
        Synthetic => "synthetic",
        ElevenLabs => "elevenLabs",
        Warp => "warp",
        Windsurf => "windsurf",
        Bifrost => "bifrost",
        Chutes => "chutes",
        LongCat => "longCat",
        ZoomMate => "zoomMate",
        NotionAi => "notionAI",
        IbmBob => "ibmBob",
        NousPortal => "nousPortal",
        RaycastAi => "raycastAI",
        GitKraken => "gitKraken",
        XKiro => "xKiro",
        Abacus => "abacus",
        Moonshot => "moonshot",
        Hyper => "hyper",
        AtlasCloud => "atlasCloud",
        Poe => "poe",
        Venice => "venice",
        OpenAiPlatform => "openAIPlatform",
        Amp => "amp",
        Zed => "zed",
        Sakana => "sakana",
        Mistral => "mistral",
        Codebuff => "codebuff",
        LlmProxy => "llmProxy",
        LiteLlm => "liteLLM",
        Aixy => "aixy",
        Neuralwatt => "neuralwatt",
        ClawRouter => "clawRouter",
        ZenMux => "zenMux",
        V0 => "v0",
        DevPass => "devPass",
        Perplexity => "perplexity",
        Manus => "manus",
        HuggingFace => "huggingFace",
        DeepInfra => "deepInfra",
        XaiApi => "xaiAPI",
        Replicate => "replicate",
        TypeSafe => "typeSafe",
        VercelAiGateway => "vercelAIGateway",
        Extension => "extension",
    }
}
