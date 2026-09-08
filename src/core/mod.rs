//! 自动战斗引擎。
//!
//! 分层：`engine`/`operation`/`system`/`query`/`component` 是基础层——
//! 统一运行、候选排序、受控关联提交与通知；`business` 是业务层——
//! 伤害、死亡、胜负、技能与流程规则。基础层不认识任何业务类型。

pub mod business;
pub mod component;
pub mod engine;
pub mod event;
pub mod log;
pub mod operation;
pub mod player;
pub mod query;
pub mod state;
pub mod system;

pub type PlayerId = usize;
pub type ComponentId = usize;

pub use component::{Component, DestructionReason};
pub use engine::{BattleEngine, BattleResult, MAX_ROUNDS};
pub use event::{Checkpoint, Event, EventKind};
pub use operation::{
    completed, completed_with, skipped, ActionContext, AddPlayerOperation, ChangeSet,
    DestroyedInfo, EmitEvent, HpChange, Operation, OperationError, OperationOutcome,
    OperationResult, OperationValue, RemoveDataOperation, RemovePlayerOperation, SubmissionResult,
};
pub use system::{Destruction, EventEnvelope, Priority, ReactionTarget, System};

// 业务类型在根模块保持可用；基础实现不反向依赖业务类型。
pub use business::attack::{Attack, AttackOperation};
pub use business::damage::{Damage, DamageDraft};
pub use business::death::DeathOperation;
pub use business::flow::{DuelOperation, DuelRunner, RoundOperation, TurnOperation};
pub use business::heal::{Heal, HpModifier};
pub use business::install_default_rules;
pub use business::skills::{Abilities, Ability, Combo, ComboOperation};
pub use business::taunt::{attach_taunt, TauntData};
