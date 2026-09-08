//! 发布事件、增删玩家与移除数据的基础操作。

use super::{completed, skipped, ExecutionContext, Operation, OperationError, OperationOutcome};
use crate::core::{buff_data::DestructionReason, event::Event, BuffId, PlayerId};

#[derive(Debug)]
pub struct EmitEvent(pub Event);

impl Operation for EmitEvent {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.publish(self.0)?;
        completed()
    }
}

#[derive(Debug)]
pub struct AddPlayerOperation(pub crate::core::player::Player);

impl Operation for AddPlayerOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.add_player(self.0)?;
        completed()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RemovePlayerOperation(pub PlayerId);

impl Operation for RemovePlayerOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        if context.remove_player(self.0)? {
            completed()
        } else {
            skipped()
        }
    }
}

#[derive(Debug)]
pub struct RemoveDataOperation {
    pub buff_id: BuffId,
    pub reason: DestructionReason,
}

impl Operation for RemoveDataOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        if context.remove_data(self.buff_id, self.reason)? {
            completed()
        } else {
            skipped()
        }
    }
}
