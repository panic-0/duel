//! 自动战斗引擎。
//!
//! 分层：`world`/`operation`/`system`/`query`/`buff_data` 是基础层——
//! 统一运行、候选排序、受控关联提交与通知；`business` 是业务层——
//! 伤害、死亡、胜负、技能与流程规则。基础层不认识任何业务类型。

pub mod buff_data;
pub mod business;
pub mod event;
pub mod log;
pub mod operation;
pub mod player;
pub mod query;
pub mod state;
pub mod system;
pub mod world;

pub type PlayerId = usize;
pub type BuffId = usize;

pub use buff_data::{BuffData, DestructionReason};
pub use event::{Checkpoint, Event, EventType};
pub use operation::{
    completed, completed_with, skipped, AddPlayerOperation, ChangeSet, DestroyedInfo, EmitEvent,
    ExecutionContext, HpChange, Operation, OperationContext, OperationError, OperationOutcome,
    OperationResult, OperationValue, RemoveDataOperation, RemovePlayerOperation, SubmissionResult,
};
pub use system::{Destruction, Fact, NoticeKind, Priority, Subject, System};
pub use world::{GameResult, World, MAX_ROUNDS};

// 业务类型在根模块保持可用；基础实现不反向依赖业务类型。
pub use business::attack::{Attack, AttackOperation};
pub use business::damage::{Damage, DamageContext};
pub use business::death::DeathOperation;
pub use business::flow::{DuelOperation, DuelRunner, RoundOperation, TurnOperation};
pub use business::heal::{Heal, HpModifier};
pub use business::install_default_rules;
pub use business::skills::{Abilities, Ability};
