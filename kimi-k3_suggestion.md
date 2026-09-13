# regexr-rust-cn 项目评审意见

> 评审范围：server 端全部 Rust 代码（约 1354 行）、测试、CI/CD、文档、scripts。
> 评审时测试套件全绿（21 单测 + 6 fixture 回放 + 2 proptest）。
> 评审日期：2026-09-13

## 总体判断

这是一个完成度远高于平均水平的项目，核心原因是**它清楚自己是什么**：不做产品、不做平台，就是"把攻击面收敛到一个端点的内网正则验证工具"。所有架构决策都服务于这一个目标，没有一处过度设计。

## 做得好的地方

1. **安全模型诚实且可审计**。`server/src/config.rs` 集中全部加固常量供安全部门逐条对照；`server/src/lib.rs:31` 的注释明说"wall-clock timeout 打断不了运行中的 pcre2_match FFI，真正的防线是 match/depth limit"——不夸大自身防御能力。
2. **unsafe 纪律好**。全部 FFI 集中在 `server/src/solve/engine.rs`，Drop 完整覆盖 4 个 PCRE2 指针，`unsafe impl Send` 的理由写在注释里（单线程 spawn_blocking 内创建-使用-销毁）。逐条核查未发现 UB。
3. **UTF-16 偏移的决策比原 PHP 后端更正确**（前端 substr/length 本来就是 UTF-16 语义），且 README 如实记录了与 PHP 的系统性差异（恒 UTF 模式），不隐瞒取舍。
4. **测试分层合理**：引擎单测（含两个真实回归测试：多字节空匹配、astral 字符）+ 手写 fixture 端到端回放 + proptest 不变量 + 真实二进制冒烟。冒烟测 musl 产物补上了 `cargo test` 覆盖不到的部署层。
5. **CI/CD 是从失败中迭代出来的**。"单 job 发布避免 GitHub API 竞争"、"CI 里必须先构建前端否则 rust-embed 编不过"——这些 commit 说明管线是真跑过、真挂过、真修了的。

## 存在的问题（按严重程度排序）

### 1. PHP 对拍至今没做 —— 最大的残留风险

fixtures 是**手写**的，代表"认为 PHP 契约是什么"，而不是"PHP 实际是什么"。空匹配推进语义、`$13→$1+"3"` 的回退规则、tests 模式省略 i 字段——这些边角行为只要有一处和真实 PHP 不一致，用户在内网拿它验证 Suricata 规则时就会得到和原 regexr 不同的结果，而且这种差异**不会报错，会静默给出错误答案**。

`scripts/gen-fixtures.php` 已经备好，只差在有 PHP 的环境跑一遍：

```bash
php scripts/gen-fixtures.php server/tests/fixtures/*.json
cd server && cargo test   # 回放新生成的 fixtures，差异会直接炸出来
```

这是全项目唯一必须用"必须做"来形容的事。

### 2. `match_all` 达到上限后仍继续扫描（`server/src/solve/engine.rs:258`）

`out.len() >= MAX_MATCHES` 之后只是不 push 了，循环照样把整个 subject 扫完。1MB 文本 + 空匹配模式会产生百万次 FFI 调用，全是无用功。改成 `else { break }` 一行解决。

### 3. UTF-16 偏移转换是 O(n) per match（`server/src/solve/offsets.rs`）

`byte_to_utf16` 每次从头扫。20,000 个匹配 × 1MB 文本，最坏约 10^10 次迭代，单请求能烧掉数秒 CPU——而 10s 超时打断不了 `spawn_blocking`（代码注释里自己写的）。并发 8 + load shed 兜住了，不算漏洞，但 DEPLOY.md 里"病态正则毫秒级返回"的表述在这类输入下不成立。一遍预扫描建累积偏移表就是 O(n+m)。

### 4. `form_value` 的提前返回 bug（`server/src/http/api.rs:69`）

`pair.split_once('=')?`——body 里任何**一个**不含 `=` 的 pair 会让整个函数返回 `None`，而不是跳过该 pair。真实前端不会触发，但既然这段代码的定位就是"防御性解析"，当前行为比 skip 更脆。改成：

```rust
let Some((k, v)) = pair.split_once('=') else { continue };
```

### 5. 小问题清单

- `assembled_index` 用 `static OnceLock`，`state` 参数名不副实（第二次调用传入的 state 被静默忽略）。当前单实例无所谓，但语义不干净。
- index 每次请求都 `.to_string()` 拷贝整页 HTML（`server/src/http/static_assets.rs:99`），用 `Cow<'static, str>` 或直接存 `Bytes` 即可。
- 无编译缓存：每次击键都走一遍 compile + JIT（JIT 编译是大头）。一个 100 条的 `(pattern, flags)` LRU 缓存能砍掉大部分延迟和 CPU。内网小规模，可做可不做。
- axum 0.7 已落后一个大版本。内网部署没法靠"常升级"修漏洞，建议在 weekly CI 里加一步 `cargo audit`，重点盯 PCRE2 的 CVE（vendored 源码不会自己更新）。

## 工程层面建议

- 功能完整度实际上已经够 1.0 了。建议路径：**跑完 PHP 对拍 → 修掉 #2/#4 → 直接打 1.0**。现在的 0.1.1 低估了项目状态。
- 前端是 vendored 的老 gulp 代码库，目前"当二进制资产、绝不重构"的分寸保持得很好，请继续保持——任何"顺手优化一下前端"的冲动都会破坏这个项目最值钱的克制。
- DEPLOY.md 的安全对照表是这份项目对安全部门最有说服力的资产，修完 #2/#3 后记得同步更新"病态正则毫秒级返回"那句的措辞。

## 一句话结论

范围控制、安全设计、文档诚实度都对的少见项目；代码里没有发现安全性错误。真正要紧的事只有一件：**把 PHP 对拍跑了**。手写 fixtures 是项目目前唯一的"未验证假设"，而它恰恰承载着项目的核心价值——和 PHP 后端行为一致。
