//! 操作协议及其受控执行接口；具体实现按职责分文件。

mod builtins;
mod changes;
mod context;

pub use builtins::{AddPlayerOperation, EmitEvent, RemoveDataOperation, RemovePlayerOperation};
pub(crate) use changes::HpRequest;
pub use changes::{ChangeSet, DestroyedInfo, HpChange, SubmissionResult};
pub use context::ActionContext;

use std::{any::Any, fmt::Debug};

/// 操作返回的值。在执行器边界做类型擦除，让互不相关的操作共用同一个迭代工作栈。
pub type OperationValue = Box<dyn Any>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationError {
    Invalid(String),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationResult {
    Completed,
    Skipped,
}

pub type OperationOutcome = (OperationResult, Option<OperationValue>);

pub fn completed() -> Result<OperationOutcome, OperationError> {
    Ok((OperationResult::Completed, None))
}

pub fn completed_with<T: Any>(value: T) -> Result<OperationOutcome, OperationError> {
    Ok((OperationResult::Completed, Some(Box::new(value))))
}

pub fn skipped() -> Result<OperationOutcome, OperationError> {
    Ok((OperationResult::Skipped, None))
}

/// 一段游戏行为。操作刻意保持小巧：复杂规则自己持有阶段状态，
/// 通过执行上下文调度子操作，而不是让执行器了解这条规则。
pub trait Operation: Debug + 'static {
    fn execute(
        self: Box<Self>,
        context: &mut ActionContext<'_>,
    ) -> Result<OperationOutcome, OperationError>;
}

impl Operation for Box<dyn Operation> {
    fn execute(
        self: Box<Self>,
        context: &mut ActionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        (*self).execute(context)
    }
}

pub(crate) trait ErasedOperation: Debug {
    fn execute_erased(
        self: Box<Self>,
        context: &mut ActionContext<'_>,
    ) -> Result<OperationOutcome, OperationError>;
}

impl<T: Operation> ErasedOperation for T {
    fn execute_erased(
        self: Box<Self>,
        context: &mut ActionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        self.execute(context)
    }
}
