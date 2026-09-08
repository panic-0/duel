pub mod ability;
pub mod buff;
#[doc(hidden)]
pub mod command;
pub mod event;
#[doc(hidden)]
pub mod flow;
pub mod log;
pub mod modifier;
pub mod operation;
pub mod player;
pub mod state;
pub mod world;

pub type PlayerId = usize;
pub type BuffId = usize;

pub use operation::{
    completed, completed_with, skipped, AddBuffOperation, AddPlayerOperation, Damage,
    DamageContext, DeathOperation, DuelOperation, EmitEvent, ExecutionContext, Heal, HpChange,
    Operation, OperationContext, OperationError, OperationOutcome, OperationResult, OperationValue,
    RemoveBuffOperation, RemovePlayerOperation, RoundOperation, TurnOperation,
};
