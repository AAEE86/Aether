<p align="center">
  <img src="frontend/public/aether_adaptive.svg" width="120" height="120" alt="Aether Logo">
</p>

<h1 align="center">Aether</h1>

<p align="center">
  <strong>开源 AI API 网关</strong><br>
  支持 Claude / OpenAI / Gemini 及其 CLI 客户端的统一接入层
</p>
<p align="center">
  <a href="#简介">简介</a> •
  <a href="#部署">部署</a> •
  <a href="#环境变量">环境变量</a> •
  <a href="#qa">Q&A</a>
</p>


---

## 简介

Aether 是一个自托管的 AI API 网关，为团队和个人提供多租户管理、智能负载均衡、成本配额控制和健康监控能力。通过统一的 API 入口，可以无缝对接 Claude、OpenAI、Gemini 等主流 AI 服务及其 CLI 工具。

### 页面预览

| 首页 | 仪表盘 |
|:---:|:---:|
| ![首页](docs/screenshots/home.png) | ![仪表盘](docs/screenshots/dashboard.png) |

| 健康监控 | 用户管理 |
|:---:|:---:|
| ![健康监控](docs/screenshots/health.png) | ![用户管理](docs/screenshots/users.png) |

| 提供商管理 | 使用记录 |
|:---:|:---:|
| ![提供商管理](docs/screenshots/providers.png) | ![使用记录](docs/screenshots/usage.png) |

| 模型详情 | 关联提供商 |
|:---:|:---:|
| ![模型详情](docs/screenshots/model-detail.png) | ![关联提供商](docs/screenshots/model-providers.png) |

| 链路追踪 | 系统设置 |
|:---:|:---:|
| ![链路追踪](docs/screenshots/tracing.png) | ![系统设置](docs/screenshots/settings.png) |

## 部署

### Docker Compose（推荐：预构建镜像）

```bash
# 1. 克隆代码
git clone https://github.com/fawney19/Aether.git
cd Aether

# 2. 配置环境变量
cp .env.example .env
python generate_keys.py  # 生成密钥, 并将生成的密钥填入 .env

# 3. 部署
docker compose up -d

# 4. 首次部署时, 初始化数据库
./migrate.sh

# 5. 更新
docker compose pull && docker compose up -d && ./migrate.sh
```

### Docker Compose（本地构建镜像）

```bash
# 1. 克隆代码
git clone https://github.com/fawney19/Aether.git
cd Aether

# 2. 配置环境变量
cp .env.example .env
python generate_keys.py  # 生成密钥, 并将生成的密钥填入 .env

# 3. 部署 / 更新（自动构建、启动、迁移）
./deploy.sh
```

### 本地开发

```bash
# 启动依赖
docker compose -f docker-compose.build.yml up -d postgres redis

# 后端
uv sync
./dev.sh

# 前端
cd frontend && npm install && npm run dev
```

## 环境变量

### 必需配置

| 变量 | 说明 |
|------|------|
| `DB_PASSWORD` | PostgreSQL 数据库密码 |
| `REDIS_PASSWORD` | Redis 密码 |
| `JWT_SECRET_KEY` | JWT 签名密钥（使用 `generate_keys.py` 生成） |
| `ENCRYPTION_KEY` | API Key 加密密钥（更换后需重新配置 Provider Key） |
| `ADMIN_EMAIL` | 初始管理员邮箱 |
| `ADMIN_USERNAME` | 初始管理员用户名 |
| `ADMIN_PASSWORD` | 初始管理员密码 |

### 可选配置

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `APP_PORT` | 8084 | 应用端口 |
| `API_KEY_PREFIX` | sk | API Key 前缀 |
| `LOG_LEVEL` | INFO | 日志级别 (DEBUG/INFO/WARNING/ERROR) |
| `GUNICORN_WORKERS` | 4 | Gunicorn 工作进程数 |
| `DB_PORT` | 5432 | PostgreSQL 端口 |
| `REDIS_PORT` | 6379 | Redis 端口 |

## Q&A

### Q: 如何开启/关闭请求体记录？

管理员在 **系统设置** 中配置日志记录的详细程度:

| 级别 | 记录内容 |
|------|----------|
| Base | 基本请求信息 |
| Headers | Base + 请求头 |
| Full | Headers + 请求体 |

### Q: 管理员如何给模型配置 1M上下文 / 1H缓存 能力支持?

1. **模型管理**: 给模型设置 1M上下文 / 1H缓存 的能力支持, 并配置好价格
2. **提供商管理**: 给端点添加支持该能力的密钥, 并勾选对应的能力标签

### Q: 用户如何使用 1H缓存?

- **模型级别**: 在模型管理中针对指定模型开启 1H缓存策略
- **密钥级别**: 在密钥管理中针对指定密钥使用 1H缓存策略

> **注意**: 若对密钥设置强制 1H缓存, 则该密钥只能使用支持 1H缓存的模型, 匹配提供商Key, 将会导致这个Key无法同时用于Claude Code、Codex、GeminiCLI, 因为更推荐使用模型开启1H缓存.

### Q: 如何配置负载均衡？

在管理后台 **提供商管理** 中切换调度模式:

| 模式 | 说明 | 适用场景 |
|------|------|----------|
| **提供商优先** | 按 Provider 优先级排序, 同优先级内按 Key 优先级排序, 相同优先级哈希分散 | 优先使用特定供应商 |
| **全局 Key 优先** | 忽略 Provider 层级, 所有 Key 按全局优先级统一排序, 相同优先级哈希分散 | 跨 Provider 统一调度, 最大化利用所有 Key |

### Q: 提供商免费套餐的计费模式会计入成本吗?

> **不会**。免费套餐的计费模式倍率为 0, 产生的记录不计入成本费用。

---

## 许可证

本项目采用 [Aether 非商业开源许可证](LICENSE)。允许个人学习、教育研究、非盈利组织及企业内部非盈利性质的使用；禁止用于盈利目的。商业使用请联系获取商业许可。

## 联系作者

<p align="center">
  <img src="docs/author/qq_qrcode.jpg" width="200" alt="QQ二维码">
</p>

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=fawney19/Aether&type=Date)](https://star-history.com/#fawney19/Aether&Date)


