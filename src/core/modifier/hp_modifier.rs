use super::super::operation::{ExecutionContext, Operation, OperationError, OperationResult};
use super::*;

#[derive(Debug)]
pub struct HpModifier {
    target_id: PlayerId,
    modifier: i64,
}

impl HpModifier {
    pub fn new(target_id: PlayerId, modifier: i64) -> Self {
        HpModifier {
            target_id,
            modifier,
        }
    }

    pub fn damage(target_id: PlayerId, amount: u64) -> Self {
        Self::new(target_id, -(amount as i64))
    }

    pub fn heal(target_id: PlayerId, amount: u64) -> Self {
        Self::new(target_id, amount as i64)
    }
}

impl Command for HpModifier {
    fn apply(self: Box<Self>, world: &mut World) {
        let _ = world.modify_hp(self.target_id, self.modifier);
    }
}

impl Operation for HpModifier {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<(OperationResult, Option<Box<dyn std::any::Any>>), OperationError> {
        let Some(change) = context.modify_hp(self.target_id, self.modifier)? else {
            return Ok((OperationResult::Skipped, None));
        };
        Ok((OperationResult::Completed, Some(Box::new(change))))
    }
}
