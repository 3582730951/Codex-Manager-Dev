# Codex-Manager 2.0 Cache Design

Date: 2026-04-06

## 1. 设计目标

这份文档只讨论一件事：
在不降低模型、不降低 `reasoning_effort`、不削弱工具能力的前提下，如何把上游 prompt cache 命中率做高，并把账号切换后的 replay token 成本压低。

这里的“最优”不是数学上的全局最优，而是当前可观测条件下的工程最优：

- 上游 provider 的 cache 是黑箱
- cache TTL、驱逐策略、内部落点都不可完全观测
- 同一前缀是否落到同一内部实例也不可控
- 能控制的只有：
  - prompt 的稳定性
  - 请求落在哪个账号
  - 切号频率
  - replay 的内容边界

因此，本系统采用的最优工程方案是：

- `principal-sticky`
- `generation-aware`
- `stable-prefix-first`
- `dual-candidate cache-aware routing`
- `dependency-frontier replay`
- `shared-root-prefix for subagents`

## 2. 问题定义

### 2.1 约束

- 同一 `CLI principal` 默认只能绑定一个账号
- 同一 principal 下的多 agent 不拆账号
- 只有硬失败才切换账号
- 切换后不能把内部 quota、模型漂移、账号失效原因透传给下游
- 不允许通过降模型、降推理强度来节约 token

### 2.2 优化目标

缓存设计的目标不是单独最大化 `cached_tokens`，而是联合优化以下指标：

1. 最大化 shared prefix 命中率
2. 最小化 generation 切换后的 replay token
3. 最小化因为调度漂移造成的 cache break
4. 最小化为了拦截异常而引入的额外 CPU 和流式解析成本
5. 在以上前提下保持单 CLI 粘连

### 2.3 非目标

- 不做 semantic answer cache
- 不做“语义接近就直接复用旧回答”
- 不做跨账号共享上游私有 thread state
- 不做全历史无差别回放

## 3. 核心结论

在 agent 工作流下，最强的缓存策略不是“把历史存起来”，而是：

1. 让最稳定的前缀尽量不变
2. 让同一个 principal 尽量长期留在同一个账号
3. 切号时只重放仍被未完成任务依赖的最小上下文边界
4. 把同一 CLI 下多 agent 的公共前缀统一成一份共享根前缀

一句话概括：

`尽量不切、切了少传、传时稳定、稳定内容放前面`

## 4. 缓存算法总览

缓存算法由 6 个层次组成：

1. `Stable Prefix Packing`
2. `Workflow Spine Packing`
3. `Live Tail Packing`
4. `Dependency-Frontier Replay`
5. `Shared Root Prefix for Subagents`
6. `Dual-Candidate Cache-Aware Routing`

这 6 层不是可选项，而是一个组合算法。

## 5. Prompt 结构算法

### 5.1 Stable Prefix Packing

把最稳定、复用率最高、最不应该频繁变化的内容固定放在 prompt 最前面：

- system rules
- tenant policy
- gateway policy
- tool schema
- stable workspace profile
- stable codebase profile
- principal 级静态工作流约束

目标：

- 提高最长公共前缀长度
- 避免字段顺序和注入顺序抖动导致 cache break

实现规则：

- JSON 做 canonical serialization
- tool schema 按稳定顺序排序
- 系统策略块按固定模块顺序拼接
- 不按请求时间、不按随机值生成 prefix 内容

### 5.2 Workflow Spine Packing

中间层放“任务骨架”，不是放“全部历史”。

包括：

- 当前目标
- 已确认决策
- 仍然有效的中间结论
- 仍然有效的 repo/workspace 摘要
- 仍被未完成任务依赖的工具结果摘要

更新条件：

- 只有在任务状态真正变化时才重编译
- 普通流式输出不直接污染这个层

### 5.3 Live Tail Packing

尾部只保留最活跃、最容易变化的部分：

- 最新用户轮次
- 最新 assistant 轮次
- 最新工具调用及其必要结果
- 当前未闭合的本轮事件

目标：

- 把最容易破坏 cache 的内容压到最后
- 保证前缀部分尽量保持稳定

## 6. Replay 算法

### 6.1 为什么不能全历史 replay

全历史 replay 有三个问题：

- token 成本高
- 动态内容太多，会破坏 prefix cache
- 越长越容易把临时工具输出、旧状态、已无效中间结果带回去

### 6.2 Dependency-Frontier Replay

切号或 generation 切换后，不重放全历史，而是重放“当前未完成任务仍然依赖的最小边界”。

保留：

- 仍被引用的工具结果
- 当前仍有效的决策
- 未完成任务所需的最新 workspace/repo 摘要
- 本轮继续进行所需的最近交互

丢弃：

- 已被后续总结吸收的中间结果
- 已无关的旧工具输出
- 对当前未完成目标没有依赖关系的旧轮次文本

### 6.3 Generation-Aware Replay

每次账号硬切换都产生新的 `generation`。

规则：

- 同 generation 内只追加 live tail
- generation 切换后重编译 `stable prefix + workflow spine + live tail`
- 不跨账号复用旧上游私有 thread state
- 只复用本地可验证的规范化上下文包

## 7. 多 Agent 共享缓存算法

### 7.1 Shared Root Prefix

同一 `CLI principal` 下的多 agent 共享同一个 `root prefix`：

- 所有 agent 共享同一份稳定前缀
- 所有 agent 共享同一份 workflow spine 基线
- 子 agent 只附加自己的差异尾部

这样做的原因：

- 避免每个 agent 重复携带完整系统上下文
- 提高 principal 内部 shared prefix 命中率
- 降低多 agent 并发时的总 replay token

### 7.2 为什么不拆成多账号

如果同一 principal 下多 agent 被分散到多个账号：

- prefix locality 会立刻变差
- generation replay 会明显增加
- 相同静态前缀会在多个账号重复预热

所以除非硬失败，否则不拆账号。

## 8. 路由与缓存联合优化算法

### 8.1 总原则

缓存算法不能和调度器分离。

如果调度器只追求“请求分散得好看”，而不追求 prefix locality，那么 prompt cache 一定会被打烂。

### 8.2 Dual-Candidate Cache-Aware Routing

为每个 principal 只选两个稳定候选账号：

- `candidate_1 = HRW(principal_id, model, account_id, salt_1)`
- `candidate_2 = HRW(principal_id, model, account_id, salt_2)`

然后只在这两个候选里做最终评分。

这样做的好处：

- 账号池变化时，迁移范围有界
- 不需要每次在全账号池全量排序
- 既保留 cache locality，又降低热点风险

### 8.3 路由评分函数

当前设计采用：

```text
score =
  0.35 * cache_affinity
  + 0.20 * effective_quota_headroom
  + 0.15 * health_score
  + 0.10 * egress_stability
  + 0.10 * fairness_bias
  + 0.10 * runtime_headroom
  + warp_bonus
```

其中：

- `cache_affinity`
  - 同 principal + 同账号 + 同 model：最高
  - 已有有效 lease：显著加分
  - 共享 root prefix：中等加分
- `effective_quota_headroom`
  - `min(quotaHeadroom, quotaHeadroom5h, quotaHeadroom7d)`
- `health_score`
  - 综合 auth、CF、transport、stream 失败率
- `egress_stability`
  - 当前出口槽位稳定性
- `fairness_bias`
  - 防止极端热点
- `runtime_headroom`
  - 当前账号 permit、并发、运行时余量

### 8.4 Near-Quota Guard 门控

不是所有账号都做重型缓存保护。

只有当：

- `quotaHeadroom5h < 0.30`
- 或 `quotaHeadroom7d < 0.30`

才启用更重的近配额保护：

- 更严格的首批 SSE 预检
- 更积极的失败拦截
- 更谨慎的继续占用策略

这样做的原因：

- 对健康账号保留轻路径，降低 CPU 和内存
- 对近配额账号做更强保护，减少把临界额度浪费在错误流上

## 9. 为什么这套算法比其他方案强

### 9.1 比 semantic cache 强

semantic cache 只适合“问题和答案静态可复用”的场景。

但当前是 agent 工作流：

- 工具输出会变
- 文件状态会变
- 网页状态会变
- 账号状态会变

语义接近不等于答案可复用。

### 9.2 比 round-robin 强

round-robin 对缓存最差：

- prefix 被打散
- 同 principal 漂移严重
- replay 频率高
- 同一批静态前缀反复冷启动

### 9.3 比全历史回放强

全历史回放看似最完整，实际上最浪费：

- token 更多
- 动态内容更多
- 缓存更不稳定
- TTFT 更差

### 9.4 比“切号后原样重传”强

原样重传会把本来已经失效的中间输出一起带过去，既费 token，也破坏 prefix 局部性。

## 10. 论文、文章与真实依据

### 10.1 OpenAI Prompt Caching

官方公开信息支持以下结论：

- 缓存的是 `longest previously computed prefix`
- 从 `1024 tokens` 开始
- 之后以 `128-token` 粒度增长
- 可通过 `usage.prompt_tokens_details.cached_tokens` 观测
- 一般在 `5-10 分钟` 不活跃后清理，最长不超过 `1 小时`

这直接支持：

- 稳定前缀前置
- principal 粘连
- prompt_cache_key 稳定化
- 不频繁重排系统块

参考：

- https://openai.com/index/api-prompt-caching/

### 10.2 Preble

论文核心结论：

- prompt sharing 和负载均衡必须联合优化
- 只做均衡、不做 cache locality 会明显损失性能

文中报告的代表性结果：

- average latency 改善约 `1.5x - 14.5x`
- p99 latency 改善约 `2x - 10x`

这支持：

- cache-aware scheduling 必须是主策略
- 调度器不能只看公平，不看共享前缀

参考：

- Preble: Efficient Distributed Prompt Scheduling for LLM Serving
- https://arxiv.org/abs/2407.00023

### 10.3 DualMap

论文核心结论：

- 双哈希映射 + 两候选选择，能在 cache affinity 和负载均衡之间取得更稳定的平衡

文中代表性结果：

- 在相同 TTFT SLO 下，有效请求容量可提升到约 `2.25x`

这支持：

- dual-candidate routing
- bounded remapping
- principal 级别而不是 request 级别的稳定放置

参考：

- DualMap: Enabling Both Cache Affinity and Load Balancing for Distributed LLM Serving
- https://arxiv.org/abs/2602.06502

### 10.4 Don’t Break the Cache

论文核心结论：

- 长链路 agent 任务里，prompt caching 是有效的
- 但 naive full-context caching 可能增加延迟
- 动态内容、尤其是工具结果，会显著破坏缓存收益

文中代表性结果：

- API cost 降低约 `41% - 80%`
- TTFT 改善约 `13% - 31%`

这支持：

- dependency-frontier replay
- stable prefix / live tail 分层
- 不全量回传动态工具结果

参考：

- Don't Break the Cache: An Evaluation of Prompt Caching for Long-Horizon Agentic Tasks
- https://arxiv.org/abs/2601.06007

### 10.5 Helium / Agentic Workflow Serving

论文核心结论：

- agent workflow 应按跨调用工作流整体优化，而不是把每个请求当成完全独立事件
- overlapping prompts 和 intermediate results 会在工作流中反复出现

文中代表性结果：

- 最高约 `1.56x` speedup

这支持：

- principal / workflow / generation 应成为缓存调度的基本单位
- shared root prefix 是必要设计

参考：

- Efficient LLM Serving for Agentic Workflows: A Data Systems Perspective
- https://arxiv.org/abs/2603.16104

### 10.6 Ray Prefix-Aware Routing

官方文档的关键结论：

- shared prefix 多时，prefix-aware routing 能提升 cache locality
- 负载平衡不足时，再退回两候选类策略

这支持：

- prefix-aware first
- load-aware second
- 不追求脱离缓存语义的“绝对均衡”

参考：

- https://docs.ray.io/en/latest/serve/llm/user-guides/prefix-aware-routing.html

## 11. 当前实现与这份设计的映射

当前仓库已经实现的部分：

- `principal` 粘连租约
- `generation` 切换
- `responses-first` 统一内核
- 近配额门控
- 隐藏 quota / model drift / SSE failure 拦截
- 基于 principal 的路由评分
- 上下文摘要持久化

仍未完全完成的部分：

- 真实 OpenAI upstream 的 `cached_tokens` 长期采样
- shared root prefix 命中率真实统计
- dependency-frontier replay 与全历史 replay 的成本对比报表
- 真实 workload 下的 TTFT 改善量化

## 12. 指标体系

必须持续采样以下指标：

- `cached_tokens`
- `prefix_hit_ratio`
- `replay_token_ratio`
- `generation_switch_replay_tokens`
- `principal_stickiness_ratio`
- `account_churn_rate`
- `ttft_p50`
- `ttft_p95`
- `ttft_p99`
- `near_quota_guard_trigger_rate`

建议附加指标：

- `shared_root_prefix_reuse_ratio`
- `workflow_spine_recompile_rate`
- `live_tail_growth_rate`
- `false_failover_rate`

## 13. 实验矩阵

### 13.1 基线对比

要做 4 组对比：

1. `principal-sticky` vs `round-robin`
2. `dependency-frontier replay` vs `full-history replay`
3. `shared-root-prefix` vs `independent-agent-prefix`
4. `dual-candidate routing` vs `global greedy routing`

### 13.2 对比指标

- `cached_tokens`
- `TTFT`
- `replay token`
- `account churn`
- `failover success rate`
- `total prompt token`

### 13.3 必测边界条件

- 同一 CLI 长链路任务
- 同一 CLI 多 agent 并发
- quota 临界但未耗尽
- quota 直接耗尽
- model drift
- auth 失效
- direct -> warp
- warp -> cooldown -> recover

## 14. 当前未完成验收

- 还没有用真实 OpenAI upstream 做 `cached_tokens` 长时间采样
- 还没有用真实线上 workload 跑完整基线对比
- 还没有生成 `principal-sticky vs round-robin` 的量化报表
- 还没有生成 `dependency-frontier replay vs full-history replay` 的量化报表
- 还没有生成 `shared-root-prefix` 的真实命中率报表
- 还没有在真实 OpenAI/Codex 生产环境做长期缓存收益追踪

## 15. 最终结论

当前最强的缓存设计，不是“缓存更多内容”，而是“稳定真正应该稳定的那一部分”。

如果用一句工程判断来概括：

- 最优缓存命中率来自 `稳定前缀`
- 最优切号恢复来自 `最小必要 replay`
- 最优资源占用来自 `把重保护只用在 near-quota 账号上`
- 最优整体效果来自 `缓存算法和调度算法一体化`
