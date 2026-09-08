use super::operation::{ExecutionContext, Operation, OperationError, OperationResult};
use super::{buff::Buff, event::Event, player::Player, world::World, BuffId, PlayerId};

/// 迁移适配：仅为源代码兼容保留。新规则必须实现 [`crate::core::Operation`]，
/// 并且只在旧 apply_event 批次入口生效。
pub trait Command: std::fmt::Debug {
    fn apply(self: Box<Self>, world: &mut World);
}

#[derive(Debug, Default)]
pub struct Commands {
    pub commands: Vec<Box<dyn Command>>,
}

impl Commands {
    pub fn push<T: Command + 'static>(&mut self, command: T) {
        self.commands.push(Box::new(command));
    }
}

#[derive(Debug)]
pub struct AddPlayer {
    pub player: Player,
}

impl Command for AddPlayer {
    fn apply(self: Box<Self>, world: &mut World) {
        world.add_player(self.player);
    }
}

#[derive(Debug)]
pub struct RemovePlayer {
    pub id: PlayerId,
}

impl Command for RemovePlayer {
    fn apply(self: Box<Self>, world: &mut World) {
        world.remove_player(self.id);
    }
}

#[derive(Debug)]
pub struct AddBuff {
    pub buff: Box<dyn Buff>,
}

impl Command for AddBuff {
    fn apply(self: Box<Self>, world: &mut World) {
        world.add_buff(self.buff);
    }
}

#[derive(Debug)]
pub struct RemoveBuff {
    pub id: BuffId,
}

impl Command for RemoveBuff {
    fn apply(self: Box<Self>, world: &mut World) {
        world.remove_buff(self.id);
    }
}

#[derive(Debug)]
pub struct ApplyEvent {
    pub event: Event,
}

impl Command for ApplyEvent {
    fn apply(self: Box<Self>, world: &mut World) {
        world.queue_event(self.event);
    }
}

impl Operation for AddPlayer {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<super::operation::OperationOutcome, OperationError> {
        context.add_player(self.player);
        Ok((OperationResult::Completed, None))
    }
}

impl Operation for RemovePlayer {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<super::operation::OperationOutcome, OperationError> {
        let result = context.remove_player(self.id);
        Ok((
            if result.is_some() {
                OperationResult::Completed
            } else {
                OperationResult::Skipped
            },
            None,
        ))
    }
}

impl Operation for AddBuff {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<super::operation::OperationOutcome, OperationError> {
        context.add_buff(self.buff);
        Ok((OperationResult::Completed, None))
    }
}

impl Operation for RemoveBuff {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<super::operation::OperationOutcome, OperationError> {
        let result = context.remove_buff(self.id);
        Ok((
            if result.is_some() {
                OperationResult::Completed
            } else {
                OperationResult::Skipped
            },
            None,
        ))
    }
}

impl Operation for ApplyEvent {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<super::operation::OperationOutcome, OperationError> {
        context.publish(self.event);
        Ok((OperationResult::Completed, None))
    }
}
