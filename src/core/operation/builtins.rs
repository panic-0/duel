//! 发布事件、增删玩家与移除数据的基础操作。

use super::{completed, skipped, ActionContext, Operation, OperationError, OperationOutcome};
use crate::core::{component::DestructionReason, event::Event, ComponentId, PlayerId};

#[derive(Debug)]
pub struct EmitEvent(pub Event);

impl Operation for EmitEvent {
    fn execute(
        self: Box<Self>,
        context: &mut ActionContext<'_>,
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
        context: &mut ActionContext<'_>,
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
        context: &mut ActionContext<'_>,
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
    pub buff_id: ComponentId,
    pub reason: DestructionReason,
}

impl Operation for RemoveDataOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ActionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        if context.remove_component(self.buff_id, self.reason)? {
            completed()
        } else {
            skipped()
        }
    }
}
