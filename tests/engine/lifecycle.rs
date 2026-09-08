//! 数据的 owner 依赖、销毁事实与生命周期校验。

use crate::support::{BelovedMark, CrossRoleShield, DependencyCounter};
use duel::core::{
    component::DestructionReason,
    event::{Event, EventKind},
    install_default_rules,
    operation::{
        completed, ActionContext, EmitEvent, Operation, OperationError, OperationOutcome,
        RemoveDataOperation, RemovePlayerOperation,
    },
    player::Player,
    query::Query,
    system::{Destruction, EventEnvelope, Priority, ReactionTarget, System},
    BattleEngine, ComponentId, Damage, PlayerId,
};
use std::{
    any::Any,
    cell::{Cell, RefCell},
    rc::Rc,
};

/// 记录销毁事实（身份、原因）的探针 System。
#[derive(Debug)]
struct DestructionSpy {
    seen: Rc<RefCell<Vec<(ComponentId, DestructionReason)>>>,
}

impl System for DestructionSpy {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::ComponentDestroyed, Priority::Default)]
    }
    fn candidates(&self, fact: &EventEnvelope<'_>, _query: &Query<'_>) -> Vec<ReactionTarget> {
        matches!(fact, EventEnvelope::Destroyed(_))
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        if let EventEnvelope::Destroyed(Destruction {
            buff_id, reason, ..
        }) = fact
        {
            self.seen.borrow_mut().push((*buff_id, *reason));
        }
        Ok(vec![])
    }
}

#[derive(Debug)]
struct GuardShield;

#[test]
fn public_remove_player_operation_destroys_owner_dependencies() {
    let mut world = BattleEngine::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    world.add_player(Player::new("B".into(), 10, 0));
    let shield = world.attach_component(Some(a), GuardShield);
    let seen = Rc::new(RefCell::new(Vec::new()));
    world.register_system(DestructionSpy { seen: seen.clone() });

    world
        .execute(RemovePlayerOperation(a))
        .expect("公开移除应正常结算");

    assert!(world.player(a).is_none());
    assert!(
        seen.borrow()
            .iter()
            .any(|(id, reason)| *id == shield && *reason == DestructionReason::OwnerDeath),
        "公开移除必须与 owner 依赖销毁共同提交并产生销毁事实"
    );
    assert!(
        world.component::<GuardShield>(shield).is_none(),
        "owner 被移除后，其依赖数据不得继续有效"
    );
}

/// owner 依赖数据：owner 死亡时随同销毁。
#[derive(Debug)]
struct SourceData {
    owner: PlayerId,
}

/// 试图附加到已死亡 owner 的依赖数据：验证其不会成为有效实例。
/// （附加目标在此用例中无关紧要，附加动作本身会被生命周期校验拒绝。）
#[derive(Debug)]
struct AttachedData;

#[derive(Debug)]
struct AttachAfterOwnerRemoval {
    owner: PlayerId,
}

impl Operation for AttachAfterOwnerRemoval {
    fn execute(
        self: Box<Self>,
        context: &mut ActionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.attach_component(Some(self.owner), AttachedData)?;
        completed()
    }
}

#[derive(Debug)]
struct RemoveThenAttach;

impl System for RemoveThenAttach {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::RoundStart, Priority::Default)]
    }

    fn candidates(&self, _fact: &EventEnvelope<'_>, query: &Query<'_>) -> Vec<ReactionTarget> {
        query
            .components::<SourceData>()
            .iter()
            .map(|(id, _, _)| ReactionTarget::Instance(*id))
            .collect()
    }

    fn respond(
        &self,
        _fact: &EventEnvelope<'_>,
        subject: ReactionTarget,
        query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        let ReactionTarget::Instance(id) = subject else {
            return Ok(Vec::new());
        };
        let Some(data) = query.component::<SourceData>(id) else {
            return Ok(Vec::new());
        };
        Ok(vec![
            Box::new(RemovePlayerOperation(data.owner)),
            Box::new(AttachAfterOwnerRemoval { owner: data.owner }),
        ])
    }
}

#[test]
fn produced_operation_cannot_attach_component_to_deleted_owner() {
    let mut world = BattleEngine::new();
    let owner = world.add_player(Player::new("Owner".into(), 10, 0));
    let target = world.add_player(Player::new("Target".into(), 10, 0));
    let source = world.attach_component(Some(owner), SourceData { owner });
    world.register_system(RemoveThenAttach);

    // 不限定 Invalid 拒绝或业务无效跳过；只要求不留下有效的孤立依赖数据。
    let _ = world.execute(EmitEvent(Event::RoundStart { round: 1 }));

    assert!(world.player(owner).is_none());
    assert!(world.player(target).is_some());
    assert!(world.component::<SourceData>(source).is_none());
    assert!(
        world.query().components::<AttachedData>().is_empty(),
        "已死亡 owner 的新依赖不应成为有效实例；生命周期资格不豁免"
    );
}

/// 记录销毁事实（原因、数据）的探针 System。
#[derive(Debug)]
struct DestructionOrderSpy {
    seen: Rc<RefCell<Vec<(ComponentId, DestructionReason, u64)>>>,
}

impl System for DestructionOrderSpy {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::ComponentDestroyed, Priority::Default)]
    }
    fn candidates(&self, fact: &EventEnvelope<'_>, _query: &Query<'_>) -> Vec<ReactionTarget> {
        matches!(fact, EventEnvelope::Destroyed(_))
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        if let EventEnvelope::Destroyed(Destruction {
            buff_id, reason, ..
        }) = fact
        {
            let index = self.seen.borrow().len() as u64;
            self.seen.borrow_mut().push((*buff_id, *reason, index));
        }
        Ok(vec![])
    }
}

#[test]
fn owner_death_destroys_all_dependent_instances_before_any_response() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 10, 0));
    world.attach_component(Some(a), BelovedMark);
    world.attach_component(Some(a), CrossRoleShield { target_id: b });

    let after_death_count = Rc::new(Cell::new(0));
    let observed_instances = Rc::new(Cell::new(-1));
    world.register_system(DependencyCounter {
        after_death_count: after_death_count.clone(),
        observed_instances: observed_instances.clone(),
    });

    world
        .execute(Damage::new(None, a, 10))
        .expect("A 应正常死亡");

    assert_eq!(after_death_count.get(), 1);
    assert_eq!(
        observed_instances.get(),
        0,
        "死亡后通知开始时，owner=A 的依赖实例必须已经全部销毁"
    );
    assert!(world.query().components::<BelovedMark>().is_empty());
}

#[test]
fn rescue_prevents_owner_death_destruction() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 0));
    world.attach_component(Some(a), BelovedMark);
    duel::core::business::revival::register_revival_system(&mut world);
    duel::core::business::revival::attach_revival(&mut world, a);

    let seen = Rc::new(RefCell::new(Vec::new()));
    world.register_system(DestructionOrderSpy { seen: seen.clone() });

    world
        .execute(Damage::new(None, a, 10))
        .expect("致命伤害应被救回");

    assert_eq!(world.player(a).map(|p| p.hp()), Some(5));
    assert!(
        seen.borrow()
            .iter()
            .all(|(_, reason, _)| *reason != DestructionReason::OwnerDeath),
        "救回未正式死亡，不得触发 OwnerDeath 销毁（救回机会自身的 Consumed 除外）"
    );
    assert_eq!(world.query().components::<BelovedMark>().len(), 1);
}

/// 死亡爆炸数据：被销毁时对其他角色造成 `damage` 点伤害。
#[derive(Debug)]
struct ExplosionData {
    damage: u64,
}

/// 死亡爆炸 System：只对 OwnerDeath 原因的爆炸数据触发。
#[derive(Debug)]
struct ExplosionSystem {
    others: Vec<PlayerId>,
}

impl System for ExplosionSystem {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::ComponentDestroyed, Priority::Default)]
    }
    fn candidates(&self, fact: &EventEnvelope<'_>, _query: &Query<'_>) -> Vec<ReactionTarget> {
        matches!(fact, EventEnvelope::Destroyed(d) if d.reason == DestructionReason::OwnerDeath)
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        let EventEnvelope::Destroyed(Destruction { data, owner, .. }) = fact else {
            return Ok(vec![]);
        };
        let data: &(dyn Any + 'static) = *data;
        let Some(explosion) = duel::core::component::downcast_component::<ExplosionData>(data)
        else {
            return Ok(vec![]);
        };
        let Some(dead) = owner else {
            return Ok(vec![]);
        };
        Ok(self
            .others
            .iter()
            .filter(|&&other| other != *dead)
            .filter(|&&other| query.player(other).is_some_and(|p| p.is_alive()))
            .map(|&other| {
                Box::new(Damage::new(None, other, explosion.damage)) as Box<dyn Operation>
            })
            .collect())
    }
}

#[test]
fn destruction_event_envelope_carries_payload_and_only_owner_death_triggers_explosion() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    world.register_system(ExplosionSystem { others: vec![a, b] });
    // 爆炸数据带业务参数（伤害 6），owner=A。
    let explosion = world.attach_component(Some(a), ExplosionData { damage: 6 });
    // 另一份同类型数据，稍后以 Explicit 原因销毁。
    let harmless = world.attach_component(None, ExplosionData { damage: 99 });

    // 先以 Explicit 原因移除无害的那份：同类型但原因不同，不得触发爆炸。
    world
        .execute(RemoveDataOperation {
            buff_id: harmless,
            reason: DestructionReason::Explicit,
        })
        .expect("显式移除应正常结算");
    assert_eq!(
        world.player(b).map(|p| p.hp()),
        Some(20),
        "Explicit 原因不得触发死亡爆炸"
    );
    assert!(world.component::<ExplosionData>(harmless).is_none());
    assert!(world.component::<ExplosionData>(explosion).is_some());

    // A 死亡：爆炸数据以 OwnerDeath 销毁，System 从历史数据读取伤害值 6。
    world.execute(Damage::new(None, a, 10)).expect("A 应死亡");
    assert_eq!(
        world.player(b).map(|p| p.hp()),
        Some(14),
        "爆炸 System 应读取销毁事实中的业务参数（6 点伤害）"
    );
}
