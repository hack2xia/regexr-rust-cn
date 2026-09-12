# RegExr 中文版（Rust 后端私有部署）

RegExr 正则测试工具的私有部署：原版中文前端 + Rust/PCRE2 单二进制后端，
用 Rust 重写替代原 PHP 后端，大幅收窄攻击面、降低被安全扫描/渗透测试提工单的概率。
无 PHP、无 MySQL、无账号、无社区功能、无任何外部网络请求。
不依赖内网环境，公网自托管同样适用（前端 CSP、服务端防 DoS 限制等加固见 `docs/DEPLOY.md`）。

```
frontend/   RegExr 前端（GPL-3.0，见 frontend/LICENSE）
server/     Rust 后端：axum 静态托管 + PCRE2 solve API，资源内嵌（rust-embed）
docs/       部署指南（DEPLOY.md，含安全加固对照表）
scripts/    前端构建 / musl 交叉编译 / PHP fixture 生成脚本
```

## 来源与授权

- **前端**：来自 gskinner 的 [RegExr](https://github.com/gskinner/regexr/)（GPL-3.0），
  中文翻译基于 [skys215/regexr](https://github.com/skys215/regexr/) 社区 fork
  （翻译者 skys215、yaoyuan4102，署名见 `frontend/dev/src/docs/sidebar_content.js`）。
- **后端**：`server/` 为独立编写的 Rust 实现，复刻原 regexr-cn PHP 后端
  （`server/api.php`，action=`regex/solve`）的请求/响应契约，引擎为 PCRE2
  （pcre2-sys，vendored 源码随 musl 静态链接）。

GPL-3.0 合规提示：仅自用（不对外分发）不触发分发义务；若对外分发整个部署包
（含 GPL 前端），需按 GPL-3.0 提供对应源码。

## 语义说明（与原 PHP 后端的已知差异）

- 后端**始终以 PCRE2 UTF 模式**编译正则：`.`、`\w` 等按 Unicode 码点匹配，
  对中文输入行为更合理；代价是与"无 `/u` 修饰符的 PHP"（按字节匹配）存在
  系统性差异。详见 `docs/DEPLOY.md`。
- 匹配偏移量以 **UTF-16 code unit** 下发（与浏览器 JS 引擎一致），比原 PHP
  后端的 UTF-8 字符计数更贴合前端实际用法。

## 快速开始

```bash
# 开发/测试后端（需要 Rust 1.75+，PCRE2 源码由 crate vendored）
cd server && cargo test

# 完整构建与部署（前端 Node 18+，交叉编译需要 Docker）
./scripts/build-frontend.sh
./scripts/cross-build.sh     # 产出静态链接的 Linux x86_64 单二进制
```

运行与 systemd 加固配置见 [docs/DEPLOY.md](docs/DEPLOY.md)。

## API 契约

`POST /server/api.php`，body 为 `action=regex/solve&data=<encodeURIComponent(JSON)>`，
恒返回 HTTP 200 + JSON 包络（与原 PHP 后端一致）。其余 action
（账号/社区/保存等）由服务端 stub 返回空数据，保证 UI 无报错运行。
