//! System 与实例候选的统一优先级排序。

use crate::support::OrderMark;
use duel::core::{
    event::{Event, EventKind},
    operation::{EmitEvent, Operation, OperationError},
    query::Query,
    system::{EventEnvelope, Priority, ReactionTarget, System},
    BattleEngine,
};
use std::{cell::RefCell, rc::Rc};

/// 记录自己被触发顺序的假 System，用于测试事件分发
#[derive(Debug)]
struct RecordSystem {
    tag: u8,
    kind: EventKind,
    priority: Priority,
    triggered: Rc<RefCell<Vec<u8>>>,
}

impl RecordSystem {
    fn new(tag: u8, kind: EventKind, priority: Priority, triggered: Rc<RefCell<Vec<u8>>>) -> Self {
        RecordSystem {
            tag,
            kind,
            priority,
            triggered,
        }
    }
}

impl System for RecordSystem {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(self.kind, self.priority)]
    }

    fn candidates(
        &self,
        _fact: &EventEnvelope<'_>,
        _query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        vec![ReactionTarget::Standalone]
    }

    fn respond(
        &self,
        _fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        self.triggered.borrow_mut().push(self.tag);
        Ok(vec![])
    }
}

#[test]
fn higher_priority_listeners_trigger_first() {
    let triggered = Rc::new(RefCell::new(Vec::new()));
    let mut world = BattleEngine::new();
    let kind = EventKind::PlayerAttack;
    world.register_system(RecordSystem::new(
        1,
        kind,
        Priority::Default,
        triggered.clone(),
    ));
    world.register_system(RecordSystem::new(
        2,
        kind,
        Priority::Modify,
        triggered.clone(),
    ));
    world.register_system(RecordSystem::new(
        3,
        kind,
        Priority::Default,
        triggered.clone(),
    ));

    assert_eq!(world.event_registration_count(EventKind::PlayerAttack), 3);
    assert!(world.validate_registry());
    assert_eq!(world.registry_stats(), (1, 3, 3));

    world
        .execute(EmitEvent(Event::PlayerAttack {
            source_id: 0,
            target_id: 1,
            damage: 5,
        }))
        .expect("事件应正常分发");

    assert_eq!(*triggered.borrow(), vec![2, 1, 3]);
}

/// 排序探针 System：把每个实例作为候选，并记录（System 标签, 实例编号）。
#[derive(Debug)]
struct OrderSpy {
    tag: u8,
    log: Rc<RefCell<Vec<(u8, u8)>>>,
}

impl OrderSpy {
    fn new(tag: u8, log: Rc<RefCell<Vec<(u8, u8)>>>) -> Self {
        Self { tag, log }
    }
}

impl System for OrderSpy {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::RoundStart, Priority::Default)]
    }
    fn candidates(&self, _fact: &EventEnvelope<'_>, query: &Query<'_>) -> Vec<ReactionTarget> {
        query
            .components::<OrderMark>()
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
            return Ok(vec![]);
        };
        let mark = query.component::<OrderMark>(id).unwrap().0;
        self.log.borrow_mut().push((self.tag, mark));
        Ok(vec![])
    }
}

#[test]
fn unified_candidate_order_interleaves_systems_by_instance_creation_order() {
    let mut world = BattleEngine::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    world.register_system(OrderSpy::new(1, log.clone()));
    world.register_system(OrderSpy::new(2, log.clone()));
    for mark in 1..=3u8 {
        world.attach_component(None, OrderMark(mark));
    }

    world
        .execute(EmitEvent(Event::RoundStart { round: 1 }))
        .expect("事件应正常分发");

    // 排序键 (Priority, 响应主体稳定顺序, System 注册顺序)：
    // 同一实例的两个 System 相邻，实例按创建顺序交错，而不是一个 System 批量跑完。
    assert_eq!(
        *log.borrow(),
        vec![(1, 1), (2, 1), (1, 2), (2, 2), (1, 3), (2, 3)]
    );
}
