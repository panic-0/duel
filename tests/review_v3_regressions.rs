//! ECS-Style 实施验收测试：对应《实施约定》第 8 节最小验收清单。
//! 覆盖统一候选排序、owner 生命周期与作用范围分离、销毁事实（D3A）、
//! 已产生操作存活（D4）、关联提交原子可见、失败与终局边界。

use std::{any::Any, cell::Cell, cell::RefCell, rc::Rc};

use duel::core::{
    buff_data::DestructionReason,
    event::{Event, EventType},
    install_default_rules,
    operation::{
        completed, EmitEvent, ExecutionContext, Operation, OperationError, OperationOutcome,
        RemoveDataOperation,
    },
    player::Player,
    query::Query,
    system::{Destruction, Fact, NoticeKind, Priority, Subject, System},
    BuffId, Damage, DeathOperation, DuelRunner, PlayerId, World,
};

// —— 通用小工具 ——

/// 记录销毁事实（原因、数据）的探针 System。
#[derive(Debug)]
struct DestructionSpy {
    seen: Rc<RefCell<Vec<(BuffId, DestructionReason, u64)>>>,
}

impl System for DestructionSpy {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Destroyed, Priority::Default)]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &Query<'_>) -> Vec<Subject> {
        matches!(fact, Fact::Destroyed(_))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        fact: &Fact<'_>,
        _subject: Subject,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        if let Fact::Destroyed(Destruction {
            buff_id, reason, ..
        }) = fact
        {
            let index = self.seen.borrow().len() as u64;
            self.seen.borrow_mut().push((*buff_id, *reason, index));
        }
        Ok(vec![])
    }
}

// —— 验收 1：多 System、多实例、同级优先级按统一候选键执行 ——

/// 排序探针数据：每个实例一个身份编号。
#[derive(Debug)]
struct OrderMark(u8);

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
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::RoundStart), Priority::Default)]
    }
    fn candidates(&self, _fact: &Fact<'_>, query: &Query<'_>) -> Vec<Subject> {
        query
            .instances::<OrderMark>()
            .iter()
            .map(|(id, _, _)| Subject::Instance(*id))
            .collect()
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        subject: Subject,
        query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        let Subject::Instance(id) = subject else {
            return Ok(vec![]);
        };
        let mark = query.data::<OrderMark>(id).unwrap().0;
        self.log.borrow_mut().push((self.tag, mark));
        Ok(vec![])
    }
}

#[test]
fn unified_candidate_order_interleaves_systems_by_instance_creation_order() {
    let mut world = World::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    world.add_system(OrderSpy::new(1, log.clone()));
    world.add_system(OrderSpy::new(2, log.clone()));
    for mark in 1..=3u8 {
        world.add_data(None, OrderMark(mark));
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

// —— 验收 3：owner=A target=B 的效果，C 作用于 B 时按业务规则生效 ——

/// 减伤数据与规则（业务目标字段决定作用范围，owner 只管寿命）。
#[derive(Debug)]
struct CrossRoleShield {
    target_id: PlayerId,
}

#[derive(Debug)]
struct CrossRoleShieldRule;

impl duel::core::business::damage::DamageRule for CrossRoleShieldRule {
    fn candidates(
        &self,
        context: &duel::core::business::damage::DamageContext,
        query: &Query<'_>,
    ) -> Vec<Subject> {
        query
            .instances::<CrossRoleShield>()
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
        let Some(data) = query.data::<CrossRoleShield>(id) else {
            return;
        };
        if data.target_id != context.target_id {
            return;
        }
        let reduced = context.amount / 2;
        context.reduce_to(reduced);
    }
}

#[test]
fn cross_role_owner_a_target_b_shield_protects_b_and_dies_with_a() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    let c = world.add_player(Player::new("C".into(), 10, 5));
    duel::core::business::damage::register_damage_rule(
        &mut world,
        Priority::Modify,
        CrossRoleShieldRule,
    );
    // owner=A，target=B：护 C 不存在任何关系，仅按 target 匹配。
    world.add_data(Some(a), CrossRoleShield { target_id: b });

    // C 攻击 B：护盾按业务目标规则生效（伤害减半）。
    world
        .execute(Damage::new(Some(c), b, 6))
        .expect("伤害应正常结算");
    assert_eq!(
        world.get_player(b).map(|p| p.hp()),
        Some(17),
        "owner 不参与作用范围过滤，减伤应对 C→B 生效"
    );

    // A 正式死亡：owner=A 的护盾在同一次死亡提交中销毁，尽管它保护的是 B。
    world.execute(Damage::new(None, a, 10)).expect("A 应死亡");
    assert!(world.get_player(a).is_none());
    assert!(
        world.query().instances::<CrossRoleShield>().is_empty(),
        "owner 死亡后依赖实例应全部销毁"
    );

    // 护盾消失后再攻击 B：伤害全额生效。
    world
        .execute(Damage::new(Some(c), b, 6))
        .expect("伤害应正常结算");
    assert_eq!(world.get_player(b).map(|p| p.hp()), Some(11));
}

// —— 验收 4：A 正式死亡，多份依赖数据同次销毁，后续响应看不到 ——
// —— 验收 5：A 被救回则不触发 OwnerDeath 销毁 ——

#[derive(Debug)]
struct BelovedMark;

/// 在死亡后通知里统计依赖实例数量的探针。
#[derive(Debug)]
struct DependencyCounter {
    after_death_count: Rc<Cell<usize>>,
    observed_instances: Rc<Cell<i64>>,
}

impl System for DependencyCounter {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(
            NoticeKind::Event(EventType::AfterPlayerDeath),
            Priority::Default,
        )]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &Query<'_>) -> Vec<Subject> {
        matches!(fact.event(), Some(Event::AfterPlayerDeath(_)))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        self.after_death_count.set(self.after_death_count.get() + 1);
        self.observed_instances.set(
            query.instances::<BelovedMark>().len() as i64
                + query.instances::<CrossRoleShield>().len() as i64,
        );
        Ok(vec![])
    }
}

#[test]
fn owner_death_destroys_all_dependent_instances_before_any_response() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 10, 0));
    world.add_data(Some(a), BelovedMark);
    world.add_data(Some(a), CrossRoleShield { target_id: b });

    let after_death_count = Rc::new(Cell::new(0));
    let observed_instances = Rc::new(Cell::new(-1));
    world.add_system(DependencyCounter {
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
    assert!(world.query().instances::<BelovedMark>().is_empty());
}

#[test]
fn rescue_prevents_owner_death_destruction() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 0));
    world.add_data(Some(a), BelovedMark);
    duel::core::business::revival::register_revival_system(&mut world);
    duel::core::business::revival::add_revival(&mut world, a);

    let seen = Rc::new(RefCell::new(Vec::new()));
    world.add_system(DestructionSpy { seen: seen.clone() });

    world
        .execute(Damage::new(None, a, 10))
        .expect("致命伤害应被救回");

    assert_eq!(world.get_player(a).map(|p| p.hp()), Some(5));
    assert!(
        seen.borrow()
            .iter()
            .all(|(_, reason, _)| *reason != DestructionReason::OwnerDeath),
        "救回未正式死亡，不得触发 OwnerDeath 销毁（救回机会自身的 Consumed 除外）"
    );
    assert_eq!(world.query().instances::<BelovedMark>().len(), 1);
}

// —— 验收 6：销毁事实携带数据与原因 ——

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
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Destroyed, Priority::Default)]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &Query<'_>) -> Vec<Subject> {
        matches!(fact, Fact::Destroyed(d) if d.reason == DestructionReason::OwnerDeath)
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        fact: &Fact<'_>,
        _subject: Subject,
        query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        let Fact::Destroyed(Destruction { data, owner, .. }) = fact else {
            return Ok(vec![]);
        };
        let data: &(dyn Any + 'static) = *data;
        let Some(explosion) = duel::core::buff_data::downcast_data::<ExplosionData>(data) else {
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
fn destruction_fact_carries_payload_and_only_owner_death_triggers_explosion() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    world.add_system(ExplosionSystem { others: vec![a, b] });
    // 爆炸数据带业务参数（伤害 6），owner=A。
    let explosion = world.add_data(Some(a), ExplosionData { damage: 6 });
    // 另一份同类型数据，稍后以 Explicit 原因销毁。
    let harmless = world.add_data(None, ExplosionData { damage: 99 });

    // 先以 Explicit 原因移除无害的那份：同类型但原因不同，不得触发爆炸。
    world
        .execute(RemoveDataOperation {
            buff_id: harmless,
            reason: DestructionReason::Explicit,
        })
        .expect("显式移除应正常结算");
    assert_eq!(
        world.get_player(b).map(|p| p.hp()),
        Some(20),
        "Explicit 原因不得触发死亡爆炸"
    );
    assert!(world.get_data::<ExplosionData>(harmless).is_none());
    assert!(world.get_data::<ExplosionData>(explosion).is_some());

    // A 死亡：爆炸数据以 OwnerDeath 销毁，System 从历史数据读取伤害值 6。
    world.execute(Damage::new(None, a, 10)).expect("A 应死亡");
    assert_eq!(
        world.get_player(b).map(|p| p.hp()),
        Some(14),
        "爆炸 System 应读取销毁事实中的业务参数（6 点伤害）"
    );
}

// —— 验收 7（D4）：反击中来源销毁，已产生的治疗不自动取消 ——

/// 来源数据：治疗操作显式检查它是否仍存在（是否存在的判断即全部语义）。
#[derive(Debug)]
struct SourceMark;

/// 反击组合：反噬来源致死，随后提出一个无条件治疗和一个显式检查来源的治疗。
#[derive(Debug)]
struct CounterThenHeal {
    owner: PlayerId,
    ally_unconditional: PlayerId,
    ally_conditional: PlayerId,
    source_buff: BuffId,
}

#[derive(Debug)]
struct UnconditionalHeal(PlayerId);
impl Operation for UnconditionalHeal {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.submit(duel::core::ChangeSet::new().heal(self.0, 3))?;
        completed()
    }
}

#[derive(Debug)]
struct HealOnlyIfSourceExists(PlayerId, BuffId);
impl Operation for HealOnlyIfSourceExists {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        if context.data::<SourceMark>(self.1).is_none() {
            return Ok((duel::core::OperationResult::Skipped, None));
        }
        context.submit(duel::core::ChangeSet::new().heal(self.0, 3))?;
        completed()
    }
}

impl Operation for CounterThenHeal {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.execute(Damage::new(None, self.owner, 10))?;
        context.execute(UnconditionalHeal(self.ally_unconditional))?;
        context.execute(HealOnlyIfSourceExists(
            self.ally_conditional,
            self.source_buff,
        ))?;
        completed()
    }
}

#[test]
fn operation_combination_does_not_depend_on_source_survival() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let owner = world.add_player(Player::new("Owner".into(), 10, 0));
    let ally1 = world.add_player(Player::new("Ally1".into(), 20, 0));
    let ally2 = world.add_player(Player::new("Ally2".into(), 20, 0));
    // 仅用于初始化：让治疗量可见。
    world.get_player_mut(ally1).unwrap().set_hp(15);
    world.get_player_mut(ally2).unwrap().set_hp(15);
    let source_buff = world.add_data(Some(owner), SourceMark);

    world
        .execute(CounterThenHeal {
            owner,
            ally_unconditional: ally1,
            ally_conditional: ally2,
            source_buff,
        })
        .expect("组合操作应正常结算");

    assert!(world.get_player(owner).is_none(), "反噬应使来源 owner 死亡");
    assert!(
        world.get_data::<SourceMark>(source_buff).is_none(),
        "来源数据应随 owner 死亡销毁"
    );
    assert_eq!(
        world.get_player(ally1).map(|p| p.hp()),
        Some(18),
        "已产生的无条件治疗不得因来源销毁被自动取消"
    );
    assert_eq!(
        world.get_player(ally2).map(|p| p.hp()),
        Some(15),
        "显式要求来源存在的治疗版本应自行跳过"
    );
}

// —— 验收 8：关联提交的两组变化对所有响应同时可见 ——

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

// —— 验收 9：响应失败后，一切运行期写入口与通知被拒绝 ——

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
    world.add_system(FailureOn(EventType::RoundStart));

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

// —— 验收 10：终局后的新根请求被无副作用拒绝 ——

#[derive(Debug)]
struct FailureOn(EventType);

impl System for FailureOn {
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
        Err(OperationError::Failed("终局测试不应触发".into()))
    }
}

#[test]
fn terminal_world_rejects_new_root_request_without_side_effects() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    world.add_system(FailureOn(EventType::HpChanged));
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

// —— 验收 11：无默认规则的自定义死亡路径完整结束 ——

#[test]
fn custom_death_rule_without_default_rules_completes_full_lifecycle() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 10, 0));
    world.add_data(Some(a), BelovedMark);
    // 只装配自定义死亡判断，不装配默认规则。
    world.add_system(CustomDeathSystem);
    let after_death = Rc::new(Cell::new(0));
    world.add_system(DependencyCounter {
        after_death_count: after_death.clone(),
        observed_instances: Rc::new(Cell::new(-1i64)),
    });

    world
        .execute(Damage::new(None, a, 10))
        .expect("致命伤害应正常结算");

    assert!(world.get_player(a).is_none(), "自定义死亡应完成角色移除");
    assert!(
        world.query().instances::<BelovedMark>().is_empty(),
        "owner 依赖销毁由受控提交完成"
    );
    assert_eq!(after_death.get(), 1, "死亡后通知应完整发布");
    assert!(world.get_player(b).is_some());
    assert!(!world.is_end(), "未装配胜负规则时不得自行判负");
}

#[derive(Debug)]
struct CustomDeathSystem;
impl System for CustomDeathSystem {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::HpChanged), Priority::Final)]
    }
    fn candidates(&self, fact: &Fact<'_>, query: &Query<'_>) -> Vec<Subject> {
        if !matches!(fact.event(), Some(Event::HpChanged { .. })) {
            return Vec::new();
        }
        if query.state().get_players().values().any(|p| p.hp() == 0) {
            vec![Subject::Standalone]
        } else {
            Vec::new()
        }
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(query
            .state()
            .get_players()
            .iter()
            .filter(|(_, p)| p.hp() == 0)
            .map(|(&id, _)| Box::new(DeathOperation { player_id: id }) as Box<dyn Operation>)
            .collect())
    }
}

// —— 验收 2：响应中新增实例不补收当前事件，但可立即参与子事件 ——

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
