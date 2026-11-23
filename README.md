# pansou-rs

PanSou 网盘搜索聚合器的 Rust 重写版。用 Rust 重写后端，复用并**内嵌原前端**到单个二进制中。

## 特性

- **单二进制**：通过 `rust-embed` 将前端 `dist` 内嵌，运行时无需外部静态文件目录。
- **核心框架 Rust 重写**：配置、路由、JWT 认证、缓存、搜索服务、TG 频道抓取、网盘链接检测。
- **20 个爬虫插件**：统一 `SearchPlugin` trait + 异步引擎（缓存 / 限流 / 超时 / 后台合并），覆盖 JSON / JSON:API / HTML 两步 / AES-XOR 解密等多种解析模式。
- **链接检测**：支持 baidu / quark / aliyun / uc / tianyi / 123 / xunlei / 115 / mobile 九种网盘，状态分级 TTL 缓存，代理隔离。

## 构建

```bash
cd pansou-rs
cargo build --release
# 产物: target/release/pansou-rs （约 9MB，已内嵌前端）
```

国内网络环境已配置 `~/.cargo/config.toml` 使用 rsproxy 镜像加速依赖下载。

## 运行

```bash
./target/release/pansou-rs
# 默认监听 0.0.0.0:4041
```

单文件部署：只需拷贝 `target/release/pansou-rs` 到目标机器即可，前端已内嵌。

## API

| 接口 | 方法 | 说明 |
|------|------|------|
| `/api/search` | GET / POST | 搜索（`kw` 必填；`src`=all/tg/plugin；支持 `channels`/`plugins`/`cloud_types`/`res`/`ext`/`filter`） |
| `/api/check/links` | POST | 网盘链接可用性检测 |
| `/api/health` | GET | 健康检查（状态、频道数、插件列表、认证开关） |
| `/api/auth/login` | POST | 登录（认证启用时） |
| `/api/auth/verify` | POST | 验证 token |
| `/api/auth/logout` | POST | 登出 |

其它路径返回内嵌前端 SPA。

## 主要环境变量

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `PORT` | `4041` | 监听端口 |
| `CHANNELS` | 内置 108 个频道 | 逗号分隔的 TG 频道列表 |
| `CONCURRENCY` | 自动 | 默认并发数 |
| `PROXY` / `HTTP_PROXY` / `HTTPS_PROXY` | 空 | 代理地址（支持 http/socks5） |
| `CACHE_ENABLED` | `true` | 搜索缓存 |
| `ASYNC_PLUGIN_ENABLED` | `true` | 启用插件搜索 |
| `ASYNC_RESPONSE_TIMEOUT` | `4` | 插件快速响应超时（秒） |
| `PLUGIN_TIMEOUT` | `30` | 插件后台超时（秒） |
| `ENABLED_PLUGINS` | 空(全部) | 逗号分隔的插件白名单 |
| `AUTH_ENABLED` | `false` | 启用 JWT 认证 |
| `AUTH_USERS` | 空 | `user1:pass1,user2:pass2` |
| `AUTH_JWT_SECRET` | 运行时随机 | JWT 密钥 |
| `AUTH_TOKEN_EXPIRY` | `24` | token 有效期（小时） |

## 插件清单（45 个）

### 原有 20 个
- **JSON / API 类**：`jikepan`、`meitizy`、`melost`、`yunsou`、`ash`、`sdso`（AES 解密）、`yunso`（XOR 解密 + HTML 片段）、`quark4k`（Flarum JSON:API）
- **HTML 两步类**：`kkv`、`alupan`、`xiaoji`、`muou`、`duoduo`、`zxzj`、`hdmoli`、`quarktv`
- **HTML 复杂两步类**：`mikuclub`、`gaoqing888`、`panta`、`susu`（JWT 解码）

### 新增 20 个（全部实测 OK）
- **JSON / API 类**：`haisou`（两阶段）、`quarksoo`、`pansearch`（Next.js buildId）、`qiwei`（suggest + 详情）、`hunhepan`（4 镜像 × 3 页 POST）、`nyaa`（BT 磁力单步）、`u3c3`（动态 JS token + 磁力）
- **两步 / 三步 HTML 类**：`aikanzy`、`xiaozhang`（302 跳转）、`pianku`、`fox4k`（Apple CMS，排除夸克）、`mizixing`、`ypfxw`、`yulinshufa`（GBK 编解码）、`panwiki`（Discuz 三步 + 主备域）、`duanjuw`、`huban`、`shandian`（仅 UC）、`jutoushe`、`xinjuc`（仅百度）

### 新版本独有 5 个（pansou-main，旧版 pansou-master 没有，全部实测 OK）
- **JSON 一步类**：`ikantv`（api.naspt.vip，多网盘）、`quarkres`（squark.cc.cd，仅夸克）
- **JSON 两步类**：`aipan`（爱盼 movies/search → detail，多网盘 + 海报）
- **HTML 内嵌 JSON**：`haitunsou`（`const list = JSON.parse('...')`，与 yunsou 同模板）
- **HTML 一步类**：`xiaoyu`（xykmovie，base64 data-url，多网盘）

选型对比了新旧两版 zip：新版 113 个插件中有 25 个旧版不存在；排除 Cloudflare 拦截（`zlxapp`）、动态混淆/加密（`xiaokupan` 的 seroval、`yingso` 的 XOR+UTF16、`pan365` 的加密 URL）、需登录态（`woniu`）、多步详情页（`5266ys`/`xiaoyu` 之外的 HTML 类）后，取上述 5 个结构清晰且实测连通的插件。全部在 `src/plugins/new_wave.rs` 中实现并通过端到端搜索验证（返回真实网盘链接与密码）。

新增插件从实测连通可用的候选中选取，排除 `gying`（依赖 Cloudflare 绕过）、`javdb`（成人站）、`weibo`（非网盘搜索），最终选取 20 个。所有 HTML 解析均抽成同步函数，避免 `scraper::Html` 跨 `await` 导致的 future 非 `Send` 问题。

### 关于插件可用性
原项目 90 个插件中大量因站点失效 / Cloudflare 拦截 / 域名过期而不可用。本实现保留原有 20 个，并新增 20 个实测可用的插件。搜索时默认启用全部插件与频道。

## 源码结构

```
src/
├── main.rs            # 启动入口：配置→插件注册→路由→监听
├── model/             # 数据结构（与 Go 版 JSON 兼容）
├── config/            # 环境变量配置
├── util/              # HTTP 客户端 / 网盘类型识别 / 缓存 / JWT / 文本工具
├── plugin/            # SearchPlugin trait + 异步引擎 + 注册表
├── plugins/           # 20 个具体爬虫（helpers 为共享工具）
├── service/           # 搜索服务(含TG解析) / 链接检测 / 结果过滤
└── api/               # axum 路由 + 认证中间件 + 前端内嵌
```
