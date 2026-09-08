//! 终局后的根操作与排队子操作边界。

use crate::support::RuntimeFailureOn;
use duel::core::{
    event::EventType,
    operation::{completed, ExecutionContext, Operation, OperationError, OperationOutcome},
    player::Player,
    Damage, DuelRunner, World,
};
use std::{cell::Cell, rc::Rc};

#[derive(Debug)]
struct EndGameOp;

impl Operation for EndGameOp {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.end_game(duel::core::GameResult::Draw)?;
        completed()
    }
}

#[derive(Debug)]
struct SummonOp {
    summoned: Rc<Cell<bool>>,
    after: Rc<Cell<usize>>,
}

impl Operation for SummonOp {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        self.summoned.set(true);
        self.after.set(self.after.get() + 1);
        context.add_player(Player::new("迟到召唤".into(), 10, 1))?;
        completed()
    }
}

#[derive(Debug)]
struct SpawningParent {
    summoned: Rc<Cell<bool>>,
    after: Rc<Cell<usize>>,
}

impl Operation for SpawningParent {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.spawn(EndGameOp);
        context.spawn(SummonOp {
            summoned: self.summoned.clone(),
            after: self.after.clone(),
        });
        completed()
    }
}

#[test]
fn spawned_operations_are_not_started_after_terminal() {
    let mut world = World::new();
    world.add_player(Player::new("A".into(), 10, 0));
    let summoned = Rc::new(Cell::new(false));
    let after = Rc::new(Cell::new(0));

    world
        .execute(SpawningParent {
            summoned: summoned.clone(),
            after: after.clone(),
        })
        .expect("父操作应正常结算");

    assert!(world.is_end());
    assert_eq!(
        world.get_players().len(),
        1,
        "正式终局后，已排队但未开始的子操作不得启动"
    );
    assert!(!summoned.get(), "召唤操作不应被执行");
    assert_eq!(after.get(), 0);
}

#[test]
fn terminal_world_rejects_new_root_request_without_side_effects() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    world.add_system(RuntimeFailureOn(EventType::HpChanged));
    world.run_with_max_rounds(0).expect("对局应正常结束");
    assert!(world.is_end());

    let result = world.execute(Damage::new(None, a, 5));
    assert!(
        matches!(result, Err(OperationError::Invalid(_))),
        "终局后的新根请求应返回 Invalid 而不是执行或静默跳过"
    );
    assert_eq!(world.get_player(a).unwrap().hp(), 10, "不得产生状态变化");
    assert!(!world.is_operation_failed(), "正常终局不得被记为执行失败");
}
