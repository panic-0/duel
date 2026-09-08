//! 关联提交的冲突校验与原子可见性。

use duel::core::{
    event::{Event, EventType},
    operation::{
        completed, ChangeSet, ExecutionContext, Operation, OperationError, OperationOutcome,
    },
    player::Player,
    query::Query,
    system::{Fact, NoticeKind, Priority, Subject, System},
    Damage, PlayerId, World,
};
use std::{cell::Cell, rc::Rc};

#[derive(Debug)]
struct ConflictingHpSubmission {
    a: PlayerId,
    b: PlayerId,
}

impl Operation for ConflictingHpSubmission {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        let changes = ChangeSet::new().damage(self.a, 2).damage(self.b, 3);
        ctx.submit(changes)?;
        completed()
    }
}

#[test]
fn changeset_rejects_conflicting_hp_declarations() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 10, 0));

    let result = world.execute(ConflictingHpSubmission { a, b });

    assert!(
        matches!(result, Err(OperationError::Invalid(_))),
        "重复的生命声明应在写入前明确拒绝"
    );
    assert_eq!(
        world.get_player(a).unwrap().hp(),
        10,
        "被拒绝的提交不得落地"
    );
    assert_eq!(
        world.get_player(b).unwrap().hp(),
        10,
        "被拒绝的提交不得落地"
    );
}

#[derive(Debug)]
struct RemoveTwo {
    a: PlayerId,
    b: PlayerId,
}

impl Operation for RemoveTwo {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.submit(ChangeSet::new().remove_player(self.a).remove_player(self.b))?;
        completed()
    }
}

#[test]
fn multiple_player_removals_are_not_silently_overwritten() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 10, 0));

    let result = world.execute(RemoveTwo { a, b });
    let a_exists = world.get_player(a).is_some();
    let b_exists = world.get_player(b).is_some();
    let rejected_before_writes = result.is_err() && a_exists && b_exists;
    let applied_both = result.is_ok() && !a_exists && !b_exists;

    assert!(
        rejected_before_writes || applied_both,
        "可选择全部提交或写入前拒绝，但不能返回成功且只删除最后一名角色"
    );
}

/// 一次性护盾：把伤害减半并把自身标记为待消耗（减半使生命事实可见）。
#[derive(Debug)]
struct AtomicShield {
    target_id: PlayerId,
}

#[derive(Debug)]
struct AtomicShieldRule;

impl duel::core::business::damage::DamageRule for AtomicShieldRule {
    fn candidates(
        &self,
        context: &duel::core::business::damage::DamageContext,
        query: &Query<'_>,
    ) -> Vec<Subject> {
        query
            .instances::<AtomicShield>()
            .iter()
            .filter(|(_, _, data)| data.target_id == context.target_id)
            .map(|(id, _, _)| Subject::Instance(*id))
            .collect()
    }
    fn modify(
        &self,
        context: &mut duel::core::business::damage::DamageContext,
        subject: Subject,
        query: &Query<'_>,
    ) {
        let Subject::Instance(id) = subject else {
            return;
        };
        if query.data::<AtomicShield>(id).is_none() {
            return;
        }
        context.reduce_to(context.amount / 2);
        context.consume_buff(id);
    }
}

/// 生命变化响应：观察时要求“生命已变化且护盾已消失”同时成立。
#[derive(Debug)]
struct AtomicityObserver {
    target: PlayerId,
    observed_both: Rc<Cell<bool>>,
}

impl System for AtomicityObserver {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::HpChanged), Priority::Default)]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &Query<'_>) -> Vec<Subject> {
        matches!(fact.event(), Some(Event::HpChanged { .. }))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        fact: &Fact<'_>,
        _subject: Subject,
        query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        if let Some(Event::HpChanged { target_id, .. }) = fact.event() {
            if *target_id == self.target {
                let shield_gone = query.instances::<AtomicShield>().is_empty();
                self.observed_both.set(shield_gone);
            }
        }
        Ok(vec![])
    }
}

#[test]
fn associated_submission_shows_hp_change_and_consumption_together() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 5));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    duel::core::business::damage::register_damage_rule(
        &mut world,
        Priority::Modify,
        AtomicShieldRule,
    );
    world.add_data(Some(b), AtomicShield { target_id: b });

    let observed_both = Rc::new(Cell::new(false));
    world.add_system(AtomicityObserver {
        target: b,
        observed_both: observed_both.clone(),
    });

    world
        .execute(Damage::new(Some(a), b, 5))
        .expect("伤害应正常结算");

    assert_eq!(
        world.get_player(b).map(|p| p.hp()),
        Some(18),
        "护盾应把伤害减半"
    );
    assert!(
        observed_both.get(),
        "生命响应可见时，同组提交的护盾消耗必须已经完成"
    );
}
