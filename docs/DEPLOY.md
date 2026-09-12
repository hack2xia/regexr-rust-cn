# 部署指南（内网私有部署）

RegExr 中文版（regexr-cn fork）私有部署：Rust + PCRE2 单二进制后端，内嵌全部静态资源。无 PHP / MySQL / 账号 / 社区功能，无任何外部网络请求。

## 构建产物

```
regexr-server        # 单个静态链接的 Linux x86_64 二进制（内嵌前端资源）
```

## 构建步骤（在内网外的构建机上执行一次）

1. 前端构建（需要 Node 18+）：
   ```bash
   ./scripts/build-frontend.sh     # 产物同步到 server/static/（rust-embed 内嵌根）
   ```
2. Rust 交叉编译（需要 Docker）：
   ```bash
   ./scripts/cross-build.sh        # 产出 x86_64-unknown-linux-musl 静态二进制
   ```
   PCRE2 由 pcre2-sys 的 vendored 源码随 musl 一并静态链接，目标机无需任何系统库。

## 运行

```bash
# 默认监听 0.0.0.0:8080
./regexr-server

# 自定义地址
REGEXR_ADDR=127.0.0.1:9000 ./regexr-server
```

浏览器访问 `http://<host>:8080/`，切换到 PCRE 引擎即可验证 Suricata 的 PCRE 正则（服务端为 PCRE2 10.x + JIT，与 Suricata 6+ 一致）。

## systemd 服务样例

```ini
# /etc/systemd/system/regexr.service
[Unit]
Description=RegExr (private Rust deployment)
After=network.target

[Service]
ExecStart=/opt/regexr/regexr-server
Environment=REGEXR_ADDR=0.0.0.0:8080
Restart=always
RestartSec=2
User=regexr
# 加固：服务无状态、无文件写入需求
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=

[Install]
WantedBy=multi-user.target
```

## 安全加固对照（供安全部门审查）

全部常量集中于 `server/src/config.rs`，安全响应头在 `server/src/http/security.rs`：

| 项 | 配置 |
|---|---|
| 请求体大小 | 1 MiB 上限（超出 413） |
| 并发 | 8 并发上限，满载直接 503（LoadShed），不排队 |
| 请求超时 | 10s 兜底 |
| 灾难性回溯 | PCRE2 match_limit=1,000,000 + depth_limit=10,000（病态正则毫秒级返回 `infinite` 警告） |
| JIT 栈 | 64KB 起 / 1MB 上限 |
| 匹配数量 | 单请求最多 20,000 个匹配 |
| tests 模式 | 单请求最多 1,000 条 |
| pattern 长度 | 16KB 上限 |
| 持久化 | 无（无 DB、无文件写入、无 session、无日志用户内容） |
| 外部请求 | 零（无 GA / Google Fonts / 广告 / 社区 API） |
| 响应头 | CSP、X-Frame-Options: DENY、nosniff、Referrer-Policy: no-referrer、Permissions-Policy |
| 出错行为 | 恒 HTTP 200 + JSON 错误包络（与原 PHP 契约一致），无堆栈泄漏 |
| UTF 语义 | PCRE2 **恒以 UTF 模式**编译（`engine.rs`）：`.`、`\w` 等按 Unicode 码点匹配，对中文更友好。与"无 `/u` 修饰符的 PHP"（按字节匹配）存在系统性差异，与 Suricata 对拍时需注意其 PCRE 调用是否启用 UTF |

## 测试

```bash
cd server && cargo test     # 27 个测试：引擎、偏移、替换、包络、fixtures 回放、安全头
```

fixtures（`server/tests/fixtures/`）可在外部 PHP 环境用 `php scripts/gen-fixtures.php server/tests/fixtures/*.json` 按原 PHP 后端语义重新生成，用于对拍验证。
