# Duel

基于 Operation、System 和数据实例的自动战斗引擎。

## 文件组织

- `src/lib.rs`：库入口，公开 `duel::core`。
- `src/main.rs`：命令行入口；`src/cli/` 负责示例对局装配、构建器和终端日志。
- `src/core/operation/`：操作协议，以及执行上下文、关联变化声明和基础操作。
- `src/core/world/`：世界状态与装配入口；`execution.rs` 调度操作，`dispatch.rs` 分发事实，`submission.rs` 提交关联变化。
- `src/core/business/`：攻击、伤害、治疗、死亡、复活、技能、流程与胜负规则。
- `src/core/` 中的其余模块：玩家、状态、事件、日志、查询、数据实例和 System 协议。

文件拆分保留了原有的公开模块路径，例如 `duel::core::operation::ChangeSet` 和 `duel::core::world::World`。业务依赖基础执行接口，基础执行器不解释业务规则。

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
