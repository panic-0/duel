//! 数据更新的稳定身份、关联提交与类型边界。

use duel::core::{
    business::damage::{register_damage_rule, DamageDraft, DamageRule},
    event::{Event, EventKind},
    operation::{completed, ActionContext, ChangeSet, Operation, OperationError, OperationOutcome},
    player::Player,
    query::Query,
    system::{EventEnvelope, Priority, ReactionTarget, System},
    BattleEngine, ComponentId, PlayerId,
};
use std::{cell::Cell, rc::Rc};

/// 有剩余容量的一次性护盾：吸收伤害并扣减容量。
#[derive(Debug)]
struct CapacityShield {
    target_id: PlayerId,
    capacity: u64,
}

#[derive(Debug)]
struct CapacityShieldRule;

impl DamageRule for CapacityShieldRule {
    fn candidates(&self, context: &DamageDraft, query: &Query<'_>) -> Vec<ReactionTarget> {
        query
            .components::<CapacityShield>()
            .iter()
            .filter(|(_, _, data)| data.target_id == context.target_id)
            .map(|(id, _, _)| ReactionTarget::Instance(*id))
            .collect()
    }
    fn modify(&self, context: &mut DamageDraft, subject: ReactionTarget, query: &Query<'_>) {
        let ReactionTarget::Instance(id) = subject else {
            return;
        };
        let Some(data) = query.component::<CapacityShield>(id) else {
            return;
        };
        // 吸收伤害并声明容量扣减：与最终伤害一起进入受控关联提交。
        let absorbed = data.capacity.min(context.amount);
        context.reduce_to(context.amount - absorbed);
        if absorbed > 0 {
            context.update_component(
                id,
                Box::new(CapacityShield {
                    target_id: data.target_id,
                    capacity: data.capacity - absorbed,
                }),
            );
        }
    }
}

/// 生命变化与容量更新一起提交的探针操作。
#[derive(Debug)]
struct AbsorbAndUpdate {
    target: PlayerId,
    shield: ComponentId,
    new_capacity: u64,
}

impl Operation for AbsorbAndUpdate {
    fn execute(
        self: Box<Self>,
        ctx: &mut ActionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        let changes = ChangeSet::new().damage(self.target, 3).update_component(
            self.shield,
            Box::new(CapacityShield {
                target_id: self.target,
                capacity: self.new_capacity,
            }),
        );
        ctx.submit(changes)?;
        completed()
    }
}

/// 生命变化响应：读取护盾当前容量。
#[derive(Debug)]
struct CapacityObserver {
    shield: ComponentId,
    observed: Rc<Cell<u64>>,
}

impl System for CapacityObserver {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::HpChanged, Priority::Default)]
    }
    fn candidates(&self, fact: &EventEnvelope<'_>, _query: &Query<'_>) -> Vec<ReactionTarget> {
        matches!(fact.event(), Some(Event::HpChanged { .. }))
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        if let Some(data) = query.component::<CapacityShield>(self.shield) {
            self.observed.set(data.capacity);
        }
        Ok(vec![])
    }
}

#[test]
fn controlled_update_keeps_identity_and_joins_associated_submission() {
    let mut world = BattleEngine::new();
    let b = world.add_player(Player::new("B".into(), 20, 0));
    register_damage_rule(&mut world, Priority::Modify, CapacityShieldRule);
    let shield = world.attach_component(
        Some(b),
        CapacityShield {
            target_id: b,
            capacity: 10,
        },
    );

    // 关联提交：扣血与容量更新对所有响应同时可见。
    let observed = Rc::new(Cell::new(0u64));
    world.register_system(CapacityObserver {
        shield,
        observed: observed.clone(),
    });
    world
        .execute(AbsorbAndUpdate {
            target: b,
            shield,
            new_capacity: 7,
        })
        .expect("关联提交应正常结算");

    assert_eq!(observed.get(), 7, "生命响应可见时，容量更新必须已经完成");
    let data = world.component::<CapacityShield>(shield).expect("实例仍在");
    assert_eq!(data.capacity, 7, "更新保留原实例身份");
    assert_eq!(
        world.query().components::<CapacityShield>().len(),
        1,
        "更新不得销毁重建实例"
    );

    // 便捷入口：单次受控更新。
    world
        .execute(ConvenienceUpdate {
            shield,
            target: b,
            capacity: 4,
        })
        .expect("便捷更新应正常结算");
    assert_eq!(
        world.component::<CapacityShield>(shield).unwrap().capacity,
        4
    );
}

#[derive(Debug)]
struct ConvenienceUpdate {
    shield: ComponentId,
    target: PlayerId,
    capacity: u64,
}

impl Operation for ConvenienceUpdate {
    fn execute(
        self: Box<Self>,
        ctx: &mut ActionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        ctx.update_component(
            self.shield,
            CapacityShield {
                target_id: self.target,
                capacity: self.capacity,
            },
        )?;
        completed()
    }
}

#[derive(Debug)]
struct MarkerA;

#[derive(Debug)]
struct MarkerB;

#[derive(Debug)]
struct CrossTypeUpdate {
    id: ComponentId,
}

impl Operation for CrossTypeUpdate {
    fn execute(
        self: Box<Self>,
        ctx: &mut ActionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        ctx.update_component(self.id, MarkerB)?;
        completed()
    }
}

#[test]
fn update_component_rejects_cross_type_replacement() {
    let mut world = BattleEngine::new();
    let id = world.attach_component(None, MarkerA);

    let result = world.execute(CrossTypeUpdate { id });

    assert!(
        matches!(result, Err(OperationError::Invalid(_))),
        "跨类型替换应在写入前明确拒绝"
    );
    assert!(
        world.component::<MarkerA>(id).is_some(),
        "被拒绝的更新不得改动原实例"
    );
}
