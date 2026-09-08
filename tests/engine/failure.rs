//! 失败传播及失败后受控入口的拒绝行为。

use crate::support::{checkpoint, CheckpointCounter, Count, OrderMark, RuntimeFailureOn};
use duel::core::{
    event::{Event, EventType},
    operation::{
        completed, EmitEvent, ExecutionContext, Operation, OperationError, OperationOutcome,
    },
    player::Player,
    query::Query,
    system::{Fact, NoticeKind, Priority, Subject, System},
    PlayerId, World,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// 对指定事件一律失败响应的 System。
#[derive(Debug)]
struct ResourceFailureOn(EventType);

impl System for ResourceFailureOn {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(self.0), Priority::Default)]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &Query<'_>) -> Vec<Subject> {
        fact.event()
            .map(|_| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Err(OperationError::Failed("451cdb1 回归测试的故意失败".into()))
    }
}

/// 记录失败后各类资源写入结果的探针操作。
#[derive(Debug)]
struct TryResourceWrites {
    results: Rc<RefCell<Vec<String>>>,
}

#[derive(Debug, Default)]
struct ProbeResource {
    value: u64,
}

impl Operation for TryResourceWrites {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        let _ = ctx.publish(Event::RoundStart { round: 1 });
        let set = ctx.set_resource(ProbeResource { value: 42 });
        let mutable_err = { ctx.resource_mut::<ProbeResource>().is_err() };
        let default_err = { ctx.resource_mut_or_default::<ProbeResource>().is_err() };
        let mut results = self.results.borrow_mut();
        results.push(format!("set err={}", set.is_err()));
        results.push(format!("mut err={}", mutable_err));
        results.push(format!("default err={}", default_err));
        completed()
    }
}

#[test]
fn resource_writes_are_rejected_after_failure_and_allowed_when_healthy() {
    // 健康路径：资源写入可用。
    let mut healthy = World::new();
    let writes = Rc::new(RefCell::new(Vec::new()));
    healthy
        .execute(TryResourceWrites {
            results: writes.clone(),
        })
        .expect("健康路径应正常执行");
    assert_eq!(
        *writes.borrow(),
        vec![
            "set err=false".to_string(),
            "mut err=false".to_string(),
            "default err=false".to_string(),
        ]
    );
    assert_eq!(
        healthy.resource::<ProbeResource>().unwrap().value,
        42,
        "健康路径的写入应真实生效"
    );

    // 失败路径：全部拒绝。
    let mut world = World::new();
    world.add_system(ResourceFailureOn(EventType::RoundStart));
    let failed = Rc::new(RefCell::new(Vec::new()));
    let result = world.execute(TryResourceWrites {
        results: failed.clone(),
    });

    assert!(result.is_err());
    assert_eq!(
        *failed.borrow(),
        vec![
            "set err=true".to_string(),
            "mut err=true".to_string(),
            "default err=true".to_string(),
        ],
        "失败后不得保留无错误返回的资源写入旁路"
    );
    assert!(
        world.resource::<ProbeResource>().is_none(),
        "被拒绝的写入不得落地"
    );
}

#[derive(Debug)]
struct FailOperation;

impl Operation for FailOperation {
    fn execute(
        self: Box<Self>,
        _: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        Err(OperationError::Failed("回归测试的故意失败".into()))
    }
}

#[derive(Debug)]
struct FailingSystem;

impl System for FailingSystem {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::Checkpoint), Priority::Default)]
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
        Ok(vec![Box::new(FailOperation)])
    }
}

#[test]
fn inline_operation_error_reaches_root_and_stops_later_listeners() {
    let mut world = World::new();
    let count = Rc::new(Cell::new(0));
    world.add_system(FailingSystem);
    world.add_system(CheckpointCounter(count.clone()));
    let result = world.execute(EmitEvent(checkpoint()));
    assert!(result.is_err(), "嵌套 Err 被静默忽略");
    assert_eq!(count.get(), 0, "操作失败后更晚的监听者仍然运行了");
}

#[derive(Debug)]
struct Fail;

impl Operation for Fail {
    fn execute(
        self: Box<Self>,
        _: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        Err(OperationError::Failed("回归测试的故意失败".into()))
    }
}

/// 对指定事件一律失败响应的 System。
#[derive(Debug)]
struct OperationFailureOn(EventType);

impl System for OperationFailureOn {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(self.0), Priority::Default)]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        fact.event()
            .map(|_| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(Fail)])
    }
}

/// 故意忽略提交结果：即使调用方不传播，失败也必须阻止后续提交。
#[derive(Debug)]
struct ContinueAfterFailedHpChange {
    a: PlayerId,
    b: PlayerId,
}

impl Operation for ContinueAfterFailedHpChange {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        let _ = ctx.modify_hp(self.a, -1);
        let _ = ctx.modify_hp(self.b, -1);
        completed()
    }
}

#[test]
fn failed_hp_reaction_must_not_allow_later_world_mutation() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 10, 0));
    world.add_system(OperationFailureOn(EventType::HpChanged));

    let result = world.execute(ContinueAfterFailedHpChange { a, b });

    assert!(result.is_err());
    assert!(world.is_operation_failed());
    assert_eq!(
        world.get_player(a).unwrap().hp(),
        9,
        "不要求回滚第一次已提交的变化"
    );
    assert_eq!(
        world.get_player(b).unwrap().hp(),
        10,
        "错误发生后不应继续提交第二次变化"
    );
}

#[derive(Debug)]
struct ContinueChildAfterPublishError(Rc<Cell<usize>>);

impl Operation for ContinueChildAfterPublishError {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        // 故意不传播发布错误：即使调用方忽略，失败也必须阻止后续子操作。
        let _ = ctx.publish(Event::RoundStart { round: 1 });
        ctx.execute(Count(self.0.clone()))?;
        completed()
    }
}

#[test]
fn a_child_must_not_start_when_the_world_already_records_failure() {
    let mut world = World::new();
    let count = Rc::new(Cell::new(0));
    world.add_system(OperationFailureOn(EventType::RoundStart));

    let result = world.execute(ContinueChildAfterPublishError(count.clone()));

    assert!(result.is_err());
    assert_eq!(count.get(), 0, "不能先执行新子操作，再检查先前的失败标记");
}

/// 记录失败后各类受控调用结果的探针操作。
#[derive(Debug)]
struct AttemptWritesAfterFailure {
    results: Rc<RefCell<Vec<String>>>,
}

impl Operation for AttemptWritesAfterFailure {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        // publish 的错误记录到世界，不传播；后续受控入口应全部拒绝。
        let _ = ctx.publish(Event::RoundStart { round: 1 });
        let add = ctx.add_data(None, OrderMark(1));
        let remove = ctx.remove_player(0);
        let publish = ctx.try_publish(Event::RoundEnd { round: 1 });
        let mut results = self.results.borrow_mut();
        results.push(format!("add_data err={}", add.is_err()));
        results.push(format!("remove_player err={}", remove.is_err()));
        results.push(format!("publish err={}", publish.is_err()));
        // 本操作自身返回成功：根调用仍应返回已记录的错误。
        completed()
    }
}

#[test]
fn failure_rejects_every_subsequent_runtime_mutation_and_notice() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    world.add_system(RuntimeFailureOn(EventType::RoundStart));

    let results = Rc::new(RefCell::new(Vec::new()));
    let result = world.execute(AttemptWritesAfterFailure {
        results: results.clone(),
    });

    assert!(result.is_err(), "失败应传播到根调用");
    assert_eq!(
        *results.borrow(),
        vec![
            "add_data err=true".to_string(),
            "remove_player err=true".to_string(),
            "publish err=true".to_string(),
        ],
        "失败后所有运行期受控入口都应拒绝执行"
    );
    assert!(world.query().instances::<OrderMark>().is_empty());
    assert!(world.get_player(a).is_some(), "拒绝的移除不得产生状态变化");
}
