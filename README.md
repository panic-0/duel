# Duel

基于 Entity、Component、Event 与 Operation、System 的自动战斗引擎。

## 文件组织

- `src/lib.rs`：库入口，公开 `duel::core`。
- `src/main.rs`：命令行入口；`src/cli/` 负责示例对局装配、构建器和终端日志。
- `src/core/operation/`：操作协议，以及执行上下文、关联变化声明和基础操作。
- `src/core/engine/`：战斗引擎状态与装配入口；`execution.rs` 调度操作，`dispatch.rs` 分发事实，`submission.rs` 提交关联变化。
- `src/core/business/`：攻击、伤害、治疗、死亡、复活、技能、流程与胜负规则。
- `src/core/` 中的其余模块：玩家、状态、事件、日志、查询、Component 与 System 协议。

公开 API 采用 Entity/Component/Event 术语：`BattleEngine` 持有组件注册表与事件运行时，`BattleState` 提供只读战斗状态，`EventEnvelope` 把事件事实交给响应 System。业务依赖基础执行接口，基础执行器不解释业务规则。

## 测试组织

集成测试统一由 `tests/integration.rs` 加载，按验证的行为归类：

- `tests/engine/`：候选排序、事实分发、操作执行、失败与终局边界、生命变化、关联提交、数据更新和生命周期。
- `tests/business/`：攻击、伤害、护盾、死亡、复活、回合流程和技能行为。
- `tests/game/`：完整对局、默认规则和自定义流程。
- `tests/support/`：跨测试模块复用的探针、事件记录器和日志工具；场景专用辅助类型留在对应测试文件中。
- `src/core/player/tests.rs`：玩家单元测试，由玩家模块在测试构建时加载。

新增回归测试应放到对应行为模块，不再按审查轮次或 Git 提交号新建测试文件。

## 运行与检查

```sh
cargo run
cargo test --all-targets
cargo test --doc
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

按模块筛选集成测试：

```sh
cargo test --test integration engine::submission
cargo test --test integration business::revival
cargo test --test integration game::
```

## 运行模型

一次动作由 `Operation` 通过 `ActionContext` 发起。上下文先发布事件，`BattleEngine` 固定本次事件的响应候选并按优先级排序；每个响应产生的子操作及其后续事件完成后，才继续处理下一个候选。生命变化、组件销毁和关联更新在同一个提交边界写入，再向 System 暴露结果。

组件的 `owner` 只负责生命周期：owner 正式死亡时，其组件会产生销毁事件；组件的业务作用范围由组件字段和规则决定。
