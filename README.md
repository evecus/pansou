# PanSou 网盘搜索

基于 [fish2018/pansou](https://github.com/fish2018/pansou) 和 [fish2018/pansou-web](https://github.com/fish2018/pansou-web) 的前后端集成版本，将 Vue3 前端与 Go 后端合并为单一仓库，通过 GitHub Actions 自动构建发布。

## 特性

- 前后端一体：Vue3 网页界面 + Go API 服务打包为单个二进制
- 支持 70+ 搜索插件，覆盖主流网盘资源站
- 支持 100+ Telegram 频道聚合搜索
- 二级缓存（内存 + 磁盘），重复搜索极速响应
- 提供 Docker 镜像（amd64 / arm64 双架构）

## 快速开始

### Docker 部署（推荐）

```bash
docker run -d \
  --name pansou \
  -p 5566:5566 \
  -v /data/pansou/cache:/app/cache \
  --restart unless-stopped \
  evecus/pansou:latest
```

访问 `http://服务器IP:5566`

### Docker Compose

```yaml
services:
  pansou:
    image: evecus/pansou:latest
    container_name: pansou
    restart: unless-stopped
    ports:
      - "5566:5566"
    volumes:
      - /data/pansou/cache:/app/cache
```

```bash
docker compose up -d
```

### 直接运行二进制

从 [Releases](../../releases) 页面下载对应架构的二进制文件：

| 文件 | 适用平台 |
|------|---------|
| `pansou-linux-amd64` | x86_64 服务器 |
| `pansou-linux-arm64` | ARM 服务器（树莓派等） |

```bash
chmod +x pansou-linux-arm64
PORT=5566 ./pansou-linux-arm64
```

## 环境变量

### 基础配置

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `PORT` | `5566` | 服务监听端口 |
| `CACHE_PATH` | `./cache` | 缓存文件路径 |
| `CACHE_TTL` | `60` | 缓存有效期（分钟） |
| `CACHE_MAX_SIZE` | `100` | 最大缓存大小（MB） |
| `PROXY` | 无 | 代理地址，如 `socks5://127.0.0.1:1080` |

### 插件与频道

默认已内置全部插件和频道，无需额外配置。如需自定义：

| 变量 | 说明 |
|------|------|
| `ENABLED_PLUGINS` | 指定启用的插件，逗号分隔。设置后覆盖默认列表 |
| `CHANNELS` | 指定搜索的 TG 频道，逗号分隔。设置后覆盖默认列表 |

```bash
# 示例：只启用部分插件
docker run -d \
  --name pansou \
  -p 5566:5566 \
  -e ENABLED_PLUGINS=labi,zhizhen,shandian,duoduo,pansearch \
  evecus/pansou:latest
```

### 认证配置（可选，默认关闭）

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `AUTH_ENABLED` | `false` | 是否启用登录认证 |
| `AUTH_USERS` | 无 | 用户账号，格式 `user1:pass1,user2:pass2` |
| `AUTH_TOKEN_EXPIRY` | `24` | Token 有效期（小时） |
| `AUTH_JWT_SECRET` | 自动生成 | JWT 签名密钥，建议手动设置 |

```bash
docker run -d \
  --name pansou \
  -p 5566:5566 \
  -e AUTH_ENABLED=true \
  -e AUTH_USERS=admin:your_password \
  -e AUTH_JWT_SECRET=your_secret_key \
  evecus/pansou:latest
```

## API 文档

### 搜索

**POST /api/search**

```bash
curl -X POST http://localhost:5566/api/search \
  -H "Content-Type: application/json" \
  -d '{"kw": "速度与激情"}'
```

**GET /api/search**

```bash
curl "http://localhost:5566/api/search?kw=速度与激情"
```

**主要参数：**

| 参数 | 说明 |
|------|------|
| `kw` | 搜索关键词（必填） |
| `res` | 返回格式：`merge`（默认）、`all`、`results` |
| `src` | 数据来源：`all`（默认）、`tg`、`plugin` |
| `refresh` | `true` 强制刷新，不使用缓存 |
| `cloud_types` | 指定网盘类型，如 `baidu,quark,aliyun` |
| `plugins` | 指定搜索插件，逗号分隔 |
| `filter` | 过滤配置，如 `{"include":["合集"],"exclude":["预告"]}` |

### 健康检查

```bash
curl http://localhost:5566/api/health
```

## 从源码构建

```bash
# 克隆仓库
git clone https://github.com/evecus/pansou.git
cd pansou

# 构建前端
cd frontend && npm ci && npm run build && cd ..

# 编译二进制
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 \
  go build -ldflags="-s -w" -o pansou-linux-amd64 .
```

或直接推送到 `main` 分支，GitHub Actions 自动构建并发布。

## 支持的网盘类型

百度网盘、阿里云盘、夸克网盘、天翼云盘、UC网盘、移动云盘、115网盘、PikPak、迅雷网盘、123网盘、磁力链接、电驴链接

## License

MIT
