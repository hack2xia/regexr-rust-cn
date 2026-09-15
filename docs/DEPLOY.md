# 部署指南（内网私有部署）

RegExr 中文版（regexr-cn fork）私有部署：Rust + PCRE2 单二进制后端，内嵌全部静态资源。无 PHP / MySQL / 账号 / 社区功能，无任何外部网络请求。

## 构建产物

```
regexr-server        # 单个静态链接的 Linux x86_64 二进制（内嵌前端资源）
regexr-server        # macOS x86_64 / aarch64 / universal 二进制（见下文 macOS 部署）
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

## macOS 部署（launchd）

macOS 无 systemd，用 launchd 常驻。二进制获取二选一：

```bash
# 1) 本机构建（rustup 环境产出双架构 + universal 二进制）
./scripts/build-macos.sh            # 产物： server/target/universal/regexr-server

# 2) 从 GitHub Releases 下载 macOS 目标的 tar.gz 解压
```

安装：

```bash
sudo mkdir -p /opt/regexr
sudo cp regexr-server /opt/regexr/
# 浏览器下载的未签名二进制可能被 Gatekeeper 拦截，清除隔离属性即可：
sudo xattr -dr com.apple.quarantine /opt/regexr/regexr-server
```

launchd 配置（系统级守护进程，等价于上文 systemd 服务；若仅需当前用户登录后运行，
把 plist 放到 `~/Library/LaunchAgents/` 并删掉 `UserName`，后续命令均不需要 sudo）：

```xml
<!-- /Library/LaunchDaemons/com.regexr.server.plist -->
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.regexr.server</string>
    <key>ProgramArguments</key>
    <array>
        <string>/opt/regexr/regexr-server</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>REGEXR_ADDR</key>
        <string>0.0.0.0:8080</string>
    </dict>
    <key>UserName</key>
    <string>regexr</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>2</integer>
    <key>StandardOutPath</key>
    <string>/tmp/regexr-server.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/regexr-server.log</string>
</dict>
</plist>
```

启用 / 重启 / 停止：

```bash
sudo plutil -lint /Library/LaunchDaemons/com.regexr.server.plist   # 先校验 plist 语法
sudo launchctl bootstrap system /Library/LaunchDaemons/com.regexr.server.plist
sudo launchctl kickstart -k system/com.regexr.server    # 改配置后重启
sudo launchctl bootout system/com.regexr.server         # 停止并卸载
```

（旧式 `launchctl load -w` / `unload -w` 等价可用。）

与 systemd 样例的对应：`KeepAlive` ≈ `Restart=always`，`ThrottleInterval` ≈
`RestartSec`，`UserName` ≈ `User`。服务无状态、无文件写入需求，与 Linux 侧一致。


## 安全加固对照（供安全部门审查）

全部常量集中于 `server/src/config.rs`，安全响应头在 `server/src/http/security.rs`：

| 项 | 配置 |
|---|---|
| 请求体大小 | 1 MiB 上限（超出 413） |
| solve 并发 | 8 个 PCRE2 任务上限（信号量许可随阻塞任务全程持有），满载直接 503，不排队；静态资源不受此限制 |
| 请求超时 | 10s 兜底（超时 408）。正在运行的 `pcre2_match` FFI 调用不可中断，实际由下方 match/depth 限制兜住 |
| 灾难性回溯 | PCRE2 match_limit=1,000,000 + depth_limit=10,000（病态正则毫秒级返回 `infinite` 警告）。注意：JIT 编译失败会静默回退解释器（限制仍然生效，仅性能下降） |
| 超大输入 | 匹配数截断于 20,000 后停止扫描并返回 `"truncated": true`；捕获组单元格（匹配数×组数）预算 1,000,000、replace/list 输出 4 MiB、JSON 响应 32 MiB，超限返回明确错误而非伪装完整的截断数据；tests 超 1,000 条时响应带 `truncated_tests` 数量 |
| JIT 栈 | 64KB 起 / 1MB 上限 |
| 匹配数量 | 单请求最多 20,000 个匹配 |
| tests 模式 | 单请求最多 1,000 条 |
| pattern 长度 | 16KB 上限 |
| 持久化 | 无（无 DB、无文件写入、无 session、无日志用户内容） |
| 外部请求 | 零（无 GA / Google Fonts / 广告 / 社区 API） |
| 响应头 | CSP、X-Frame-Options: DENY、nosniff、Referrer-Policy: no-referrer、Permissions-Policy |
| 出错行为 | 业务错误恒 HTTP 200 + JSON 错误包络（与原 PHP 契约一致），无堆栈泄漏；基础设施层可能返回 413 / 503 / 408 |
| UTF 语义 | PCRE2 恒以 PHP `/u` 修饰符的完整选项集编译（`engine.rs`：`UTF + UCP + NEVER_BACKSLASH_C`，与 php-src 一致）：`.`、`\w`、`\d`、`\s`、`\b` 按 Unicode 语义匹配，`\C` 不可用。与"无 `/u` 修饰符的 PHP"（按字节匹配）存在系统性差异，与 Suricata 对拍时需注意其 PCRE 调用是否启用 UTF |
| CSP | `script-src 'self'`（无 `unsafe-inline`）：应用初始化脚本位于外部 `/server/init.js`；DOM XSS 即使绕过前端转义也无法执行内联脚本 |

## 公网暴露加固（如需公网自托管）

本服务默认按受控内网工具设计。若必须暴露公网，**不要直接把 0.0.0.0 服务暴露给
互联网**，推荐拓扑：

1. 服务只绑定 loopback：`REGEXR_ADDR=127.0.0.1:8080`；
2. 由反向代理（Caddy / nginx）或负载均衡器负责 TLS 终结、连接数限制、
   按 IP 限速与访问控制；
3. 可选：反代层加 IP 白名单 / Basic Auth，仅放行安全团队网段。

nginx 参考片段：

```nginx
server {
    listen 443 ssl;
    # ... 证书配置 ...
    location / {
        limit_req zone=regexr burst=20;   # 按 IP 限速
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header X-Forwarded-For $remote_addr;
    }
}
```

剩余风险提示：无认证的 solve 接口仍可被任何能访问到服务的人消耗 CPU/内存
（已有 match/depth 限制、资源预算与并发上限兜底），公网部署时按第 2、3 条
限流隔离是必要的。

## 测试

```bash
cd server && cargo test     # 30+ 个测试：引擎、偏移、替换、包络、fixtures 回放、proptest 性质、安全头
```

发布产物冒烟（回放全部 fixtures + 安全头检查到真实二进制上，验证部署层行为
与目标机 PCRE2 JIT 可用性，`cargo test` 覆盖不到的部署层）：

> 注：冒烟通过说明服务可用，但**不证明 JIT 一定在运行**——JIT 编译失败会
> 静默回退解释器。交叉编译的 musl 静态链接同样建议在目标机以 `ldd` /
> `file` 做最终确认（间接运行验证不等于严格 linkage 检查）。

```bash
./scripts/smoke.sh          # 默认测 cross-build.sh 的 musl 产物，无则先构建本地 debug 二进制
```

## 发布（多平台二进制）

仓库已公开，GitHub Actions 免费提供全平台构建（`.github/workflows/release.yml`）：

```bash
git tag v1.x.y && git push origin v1.x.y   # 触发发布
```

自动构建 4 个目标并在 GitHub Releases 挂出 tar.gz（内含二进制 + README + 本文档）：

| 目标 | 构建方式 | 冒烟 |
|---|---|---|
| x86_64-unknown-linux-musl | cargo-zigbuild 交叉编译（静态） | 云端执行 |
| aarch64-unknown-linux-musl | 同上（静态） | 交叉产物，目标机手动验证 |
| aarch64-apple-darwin | macos runner 原生 | 云端执行 |
| x86_64-apple-darwin | macos runner 交叉编译 | 目标机手动验证 |

推 tag 前可先在 Actions 页面手动 Run workflow（`workflow_dispatch`）验证构建，
该模式只构建 + 冒烟、不发布。macOS 双架构也可本机构建：

```bash
./scripts/build-macos.sh [--smoke]   # 无 rustup（如 MacPorts rust）时仅构建本机架构
```

Windows 暂不支持（计划 `x86_64-pc-windows-msvc`，未实现）。

## 云端发布构建（腾讯 CNB，可选）

日常质量关卡走 GitHub Actions；仓库同时配有 `.cnb.yml`，用 CNB 免费核时做
免本地 Docker 的发布构建（tag 推送时触发，CPU 4 核，约 1~2 核时/次）：

1. 把仓库导入 CNB（或 `git remote add cnb <你的仓库URL> && git push cnb main --tags`）；
2. 发布：`git tag v1.0.0 && git push origin v1.0.0`（推到 CNB 则 `git push cnb v1.0.0`）；
3. 流水线自动完成：前端构建 → musl 静态编译 → 真实二进制冒烟 → 把二进制以
   `FROM scratch` 镜像推入 CNB 制品库。取回二进制：
   ```bash
   docker pull docker.cnb.cool/<你的仓库slug>:v1.0.0
   docker create --name tmp docker.cnb.cool/<你的仓库slug>:v1.0.0
   docker cp tmp:/regexr-server ./regexr-server && docker rm tmp
   ```

仓库页面的「云原生开发」可按需启动 4 核云端开发机（Rust/Node/python3 环境预装）。

fixtures（`server/tests/fixtures/`）的期望值由**真实 PHP** 生成（本机无需装 PHP）：

```bash
# 金标准 = PHP + mbstring + 强制 /u（与服务器声明的恒 UTF 语义对齐）
docker run --rm -v "$PWD":/work -w /work php:8-cli \
    php scripts/gen-fixtures.php server/tests/fixtures/*.json
cd server && cargo test   # 回放：与 PHP 的任何行为差异会直接失败
```

脚本要点：恒加 `/u`、`PREG_UNMATCHED_AS_NULL`（未参与分组补全为 `{0,0}`）、
替换引用越界（如 `$13` 只有 1 组）展开为**空串**（PHP 真实行为，非 JS 的分解规则）、
错误按请求隔离。
