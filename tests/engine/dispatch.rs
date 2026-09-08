//! 通知的同步结算、候选快照与分发期间的实例变化。

use crate::support::{checkpoint, op_system, CheckpointCounter};
use duel::core::{
    buff_data::DestructionReason,
    event::{Event, EventType},
    operation::{
        completed, completed_with, EmitEvent, ExecutionContext, Operation, OperationError,
        OperationOutcome, RemoveDataOperation,
    },
    query::Query,
    system::{Fact, NoticeKind, Priority, Subject, System},
    BuffId, PlayerId, World,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[test]
fn removed_instance_no_longer_receives_events() {
    /// 空数据实例标记
    #[derive(Debug)]
    struct Marker;

    /// 为每个 Marker 实例产生候选并记录的 System。
    #[derive(Debug)]
    struct MarkerSpy {
        seen: Rc<RefCell<Vec<PlayerId>>>,
    }

    impl System for MarkerSpy {
        fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
            vec![(
                NoticeKind::Event(EventType::PlayerAttack),
                Priority::Default,
            )]
        }

        fn candidates(
            &self,
            _fact: &Fact<'_>,
            query: &duel::core::query::Query<'_>,
        ) -> Vec<Subject> {
            query
                .instances::<Marker>()
                .iter()
                .map(|(id, _, _)| Subject::Instance(*id))
                .collect()
        }

        fn respond(
            &self,
            _fact: &Fact<'_>,
            subject: Subject,
            _query: &duel::core::query::Query<'_>,
        ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
            if let Subject::Instance(id) = subject {
                self.seen.borrow_mut().push(id);
            }
            Ok(vec![])
        }
    }

    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut world = World::new();
    world.add_system(MarkerSpy { seen: seen.clone() });
    let marker = world.add_data(None, Marker);

    let attack = || Event::PlayerAttack {
        source_id: 0,
        target_id: 1,
        damage: 5,
    };
    world.execute(EmitEvent(attack())).expect("事件应正常分发");
    assert_eq!(*seen.borrow(), vec![marker]);

    world
        .execute(RemoveDataOperation {
            buff_id: marker,
            reason: DestructionReason::Explicit,
        })
        .expect("移除应成功");
    assert!(world.validate_registry_consistency());

    world.execute(EmitEvent(attack())).expect("事件应正常分发");
    assert_eq!(*seen.borrow(), vec![marker], "已销毁实例不应再参与事件响应");
}

#[derive(Debug)]
struct PublishThenRead(Rc<Cell<usize>>);

impl Operation for PublishThenRead {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        ctx.publish(checkpoint())?;
        completed_with(self.0.get())
    }
}

#[test]
fn n1a_publish_finishes_reactions_before_returning_to_operation() {
    let mut world = World::new();
    let count = Rc::new(Cell::new(0));
    world.add_system(CheckpointCounter(count.clone()));
    let (_, value) = world.execute(PublishThenRead(count)).expect("根操作");
    let observed = *value
        .expect("应读取到计数")
        .downcast::<usize>()
        .expect("usize");
    assert_eq!(observed, 1, "publish 在反应运行之前就返回了");
}

/// 空数据实例标记，用于验证 C3A 的新增实例语义。
#[derive(Debug)]
struct MarkerData;

/// 为每个 Marker 实例产生候选并计数的 System。
#[derive(Debug)]
struct MarkerCounter(Rc<Cell<usize>>);

impl System for MarkerCounter {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::Checkpoint), Priority::Default)]
    }
    fn candidates(&self, _fact: &Fact<'_>, query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        query
            .instances::<MarkerData>()
            .iter()
            .map(|(id, _, _)| Subject::Instance(*id))
            .collect()
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        self.0.set(self.0.get() + 1);
        Ok(vec![])
    }
}

#[derive(Debug)]
struct PublishThenAdd;

impl Operation for PublishThenAdd {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        ctx.publish(checkpoint())?;
        ctx.add_data(None, MarkerData)?;
        completed()
    }
}

#[test]
fn n1a_and_c3a_later_added_instance_does_not_receive_previously_published_event() {
    let mut world = World::new();
    let count = Rc::new(Cell::new(0));
    world.add_system(MarkerCounter(count.clone()));
    world.execute(PublishThenAdd).expect("根操作");
    assert_eq!(count.get(), 0, "新增数据实例收到了在它创建之前发布的通知");
}

#[derive(Debug)]
struct RemovePeer {
    peer: BuffId,
}

impl System for RemovePeer {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::Checkpoint), Priority::Modify)]
    }

    fn candidates(&self, fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        matches!(fact.event(), Some(Event::Checkpoint { .. }))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }

    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(RemoveDataOperation {
            buff_id: self.peer,
            reason: DestructionReason::Explicit,
        })])
    }
}

#[test]
fn audit_instance_removed_mid_dispatch_is_skipped_in_current_event() {
    let mut world = World::new();
    let count = Rc::new(Cell::new(0));
    let marker = world.add_data(None, MarkerData);
    world.add_system(MarkerCounter(count.clone()));
    world.add_system(RemovePeer { peer: marker });

    world.execute(EmitEvent(checkpoint())).expect("事件");
    assert_eq!(count.get(), 0, "轮到之前被移除的候选应被跳过");
}

#[derive(Debug)]
struct LateMarker;

#[derive(Debug)]
struct LateMarkerSpy(Rc<Cell<usize>>);

impl System for LateMarkerSpy {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::RoundEnd), Priority::Default)]
    }
    fn candidates(&self, _fact: &Fact<'_>, query: &Query<'_>) -> Vec<Subject> {
        query
            .instances::<LateMarker>()
            .iter()
            .map(|(id, _, _)| Subject::Instance(*id))
            .collect()
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        self.0.set(self.0.get() + 1);
        Ok(vec![])
    }
}

#[derive(Debug)]
struct AddMarkerThenPublishChild;

impl Operation for AddMarkerThenPublishChild {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        ctx.try_publish(Event::RoundStart { round: 1 })?;
        ctx.add_data(None, LateMarker)?;
        // 子事件：新增的实例应立即有资格参与。
        ctx.try_publish(Event::RoundEnd { round: 1 })?;
        completed()
    }
}

#[test]
fn instance_added_between_notices_skips_the_earlier_and_joins_the_later() {
    let mut world = World::new();
    let count = Rc::new(Cell::new(0));
    world.add_system(LateMarkerSpy(count.clone()));

    world
        .execute(AddMarkerThenPublishChild)
        .expect("操作应正常结算");

    assert_eq!(
        count.get(),
        1,
        "新增实例不得补收创建前的通知，但应参与其后的子通知"
    );
}

#[test]
fn event_reactions_settle_depth_first_before_their_siblings() {
    let mut world = World::new();
    let trace = Rc::new(RefCell::new(Vec::new()));
    let sink = trace.clone();
    op_system(&mut world, &[EventType::RoundStart, EventType::RoundEnd], {
        let sink = sink.clone();
        move |fact, _| {
            match fact.event() {
                Some(Event::RoundStart { round }) => sink.borrow_mut().push(*round),
                Some(Event::RoundEnd { .. }) => sink.borrow_mut().push(0),
                _ => {}
            }
            let Some(Event::RoundStart { round }) = fact.event() else {
                return vec![];
            };
            let children = match *round {
                1 => vec![2, 0],
                2 => vec![3],
                _ => vec![],
            };
            children
                .into_iter()
                .map(|round| {
                    let op: Box<dyn Operation> = if round == 0 {
                        Box::new(EmitEvent(Event::RoundEnd { round: 1 }))
                    } else {
                        Box::new(EmitEvent(Event::RoundStart { round }))
                    };
                    op
                })
                .collect()
        }
    });
    world
        .execute(EmitEvent(Event::RoundStart { round: 1 }))
        .expect("事件应正常分发");
    // 子通知先于兄弟通知完整结算：1 → 2 → 3 → 回合结束。
    assert_eq!(*trace.borrow(), vec![1, 2, 3, 0]);
}
