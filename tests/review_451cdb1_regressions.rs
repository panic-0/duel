//! 审查 451cdb1 回归测试：对应 F1–F8 与 I1 的修复行为。
//! 复活联合提交、公开删除生命周期、目标失效不消费、失败后资源写入拒绝、
//! spawn 终局检查、规则独立候选的共同排序、ChangeSet 冲突拒绝与受控数据更新。

use std::{any::Any, cell::Cell, cell::RefCell, rc::Rc};

use duel::core::{
    buff_data::DestructionReason,
    business::damage::{register_damage_rule, DamageContext, DamageRule},
    business::revival::{add_revival, register_revival_system, RevivalData},
    business::skills::{Abilities, Ability},
    event::{Event, EventType},
    install_default_rules,
    operation::{
        completed, ChangeSet, EmitEvent, ExecutionContext, Operation, OperationError,
        OperationOutcome, RemovePlayerOperation,
    },
    player::Player,
    query::Query,
    state::GameState,
    system::{Destruction, Fact, NoticeKind, Priority, Subject, System},
    BuffId, Damage, Heal, PlayerId, TurnOperation, World,
};

/// 对指定事件一律失败响应的 System。
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
        Err(OperationError::Failed("451cdb1 回归测试的故意失败".into()))
    }
}

/// 记录销毁事实（身份、原因）的探针 System。
#[derive(Debug)]
struct DestructionSpy {
    seen: Rc<RefCell<Vec<(BuffId, DestructionReason)>>>,
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
            self.seen.borrow_mut().push((*buff_id, *reason));
        }
        Ok(vec![])
    }
}

// —— F1：复活消费与恢复生命是同一次关联提交 ——

/// 观察到复活机会被 Consumed 时，对其服务对象追加 7 点伤害的 System。
#[derive(Debug)]
struct ConsumeRetaliator {
    damage: u64,
}

impl System for ConsumeRetaliator {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Destroyed, Priority::Default)]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &Query<'_>) -> Vec<Subject> {
        matches!(fact, Fact::Destroyed(d) if d.reason == DestructionReason::Consumed)
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        fact: &Fact<'_>,
        _subject: Subject,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        let Fact::Destroyed(Destruction { data, .. }) = fact else {
            return Ok(vec![]);
        };
        let data: &(dyn Any + 'static) = *data;
        let Some(revival) = duel::core::buff_data::downcast_data::<RevivalData>(data) else {
            return Ok(vec![]);
        };
        Ok(vec![Box::new(Damage::new(
            None,
            revival.player_id,
            self.damage,
        ))])
    }
}

#[test]
fn revival_consumption_and_heal_are_one_associated_submission() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 20, 0));
    register_revival_system(&mut world);
    add_revival(&mut world, a);
    world.add_system(ConsumeRetaliator { damage: 7 });

    world
        .execute(Damage::new(None, a, 20))
        .expect("致命伤害应触发救回");

    // 关联提交：消费与恢复一起完成后才开放通知 → 消费反应中的 7 点伤害落在 10 上。
    assert_eq!(
        world.get_player(a).map(|p| p.hp()),
        Some(3),
        "响应不得看到“机会已消耗但生命仍为零”的半次提交"
    );
    assert!(world.get_player(a).is_some(), "救回后不应死亡");
}

// —— F2：公开删除入口走同一条生命周期路径 ——

#[derive(Debug)]
struct GuardShield;

#[test]
fn public_remove_player_operation_destroys_owner_dependencies() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    world.add_player(Player::new("B".into(), 10, 0));
    let shield = world.add_data(Some(a), GuardShield);
    let seen = Rc::new(RefCell::new(Vec::new()));
    world.add_system(DestructionSpy { seen: seen.clone() });

    world
        .execute(RemovePlayerOperation(a))
        .expect("公开移除应正常结算");

    assert!(world.get_player(a).is_none());
    assert!(
        seen.borrow()
            .iter()
            .any(|(id, reason)| *id == shield && *reason == DestructionReason::OwnerDeath),
        "公开移除必须与 owner 依赖销毁共同提交并产生销毁事实"
    );
    assert!(
        world.get_data::<GuardShield>(shield).is_none(),
        "owner 被移除后，其依赖数据不得继续有效"
    );
}

// —— F3：伤害目标失效时不得消费声明的护盾 ——

#[derive(Debug)]
struct LateShield {
    target_id: PlayerId,
}

#[derive(Debug)]
struct LateShieldRule;

impl DamageRule for LateShieldRule {
    fn candidates(&self, context: &DamageContext, query: &Query<'_>) -> Vec<Subject> {
        query
            .instances::<LateShield>()
            .iter()
            .filter(|(_, _, data)| data.target_id == context.target_id)
            .map(|(id, _, _)| Subject::Instance(*id))
            .collect()
    }
    fn modify(&self, context: &mut DamageContext, subject: Subject, query: &Query<'_>) {
        let Subject::Instance(id) = subject else {
            return;
        };
        if query.data::<LateShield>(id).is_none() {
            return;
        }
        context.reduce_to(0);
        context.consume_buff(id);
    }
}

#[test]
fn damage_to_missing_target_does_not_consume_declared_shield() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 5));
    let b = world.add_player(Player::new("B".into(), 10, 0));
    register_damage_rule(&mut world, Priority::Modify, LateShieldRule);
    let shield = world.add_data(Some(a), LateShield { target_id: b });

    // 先移除 B（走生命周期路径），再对 B 迟到地施加伤害。
    world
        .execute(RemovePlayerOperation(b))
        .expect("移除应正常结算");
    let (result, _) = world
        .execute(Damage::new(Some(a), b, 5))
        .expect("迟到伤害应按跳过处理");

    assert_eq!(result, duel::core::OperationResult::Skipped);
    let shields: Vec<_> = world
        .query()
        .instances::<LateShield>()
        .iter()
        .map(|(id, _, _)| *id)
        .collect();
    assert_eq!(
        shields,
        vec![shield],
        "伤害不成立时，以伤害成功为前提的资源消耗不得提交"
    );
}

// —— F4：失败后资源写入被拒绝 ——

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
    world.add_system(FailureOn(EventType::RoundStart));
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

// —— F5：复活按业务 player_id 匹配，owner 只管寿命 ——

#[test]
fn revival_matches_by_business_player_id_regardless_of_owner() {
    let mut world = World::new();
    install_default_rules(&mut world);
    register_revival_system(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    // owner=A，服务对象=B：跨角色的救回机会。
    world.add_data(Some(a), RevivalData { player_id: b });

    world
        .execute(Damage::new(None, b, 20))
        .expect("致命伤害应触发救回");

    assert_eq!(
        world.get_player(b).map(|p| p.hp()),
        Some(10),
        "救回应按 player_id 匹配，即使 owner 不是被救角色"
    );
    assert!(world.get_player(a).is_some());

    // owner=A 先死亡：救回机会随之销毁，之后 B 致死时不再有救回。
    world
        .execute(Damage::new(None, a, 10))
        .expect("A 应正常死亡");
    world
        .execute(Damage::new(None, b, 20))
        .expect("第二次致命伤害");
    assert!(
        world.get_player(b).is_none(),
        "owner 死亡后救回机会已被销毁，B 应正常死亡"
    );
}

#[test]
fn ownerless_revival_still_rescues_by_player_id() {
    let mut world = World::new();
    install_default_rules(&mut world);
    register_revival_system(&mut world);
    let b = world.add_player(Player::new("B".into(), 20, 0));
    world.add_data(None, RevivalData { player_id: b });

    world
        .execute(Damage::new(None, b, 20))
        .expect("致命伤害应触发救回");

    assert_eq!(
        world.get_player(b).map(|p| p.hp()),
        Some(10),
        "owner=None 的救回数据不受任何角色死亡影响，按 player_id 正常救回"
    );
}

// —— F6：终局后已排队的 spawn 子操作不再启动 ——

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

// —— F7：DamageRule 独立候选与实例候选共用共同顺序时间线 ——

/// 减去固定数值的减伤数据。
#[derive(Debug)]
struct ReduceBy {
    target_id: PlayerId,
    amount: u64,
}

#[derive(Debug)]
struct ReduceByRule;

impl DamageRule for ReduceByRule {
    fn candidates(&self, context: &DamageContext, query: &Query<'_>) -> Vec<Subject> {
        query
            .instances::<ReduceBy>()
            .iter()
            .filter(|(_, _, data)| data.target_id == context.target_id)
            .map(|(id, _, _)| Subject::Instance(*id))
            .collect()
    }
    fn modify(&self, context: &mut DamageContext, subject: Subject, query: &Query<'_>) {
        let Subject::Instance(id) = subject else {
            return;
        };
        let Some(data) = query.data::<ReduceBy>(id) else {
            return;
        };
        context.reduce_to(context.amount.saturating_sub(data.amount));
    }
}

/// 减半的独立规则（不绑定实例）。
#[derive(Debug)]
struct HalveRule;

impl DamageRule for HalveRule {
    fn candidates(&self, _context: &DamageContext, _query: &Query<'_>) -> Vec<Subject> {
        vec![Subject::Standalone]
    }
    fn modify(&self, context: &mut DamageContext, _subject: Subject, _query: &Query<'_>) {
        context.reduce_to(context.amount / 2);
    }
}

#[test]
fn damage_rule_standalone_candidates_share_the_common_order_timeline() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    // 先创建实例（主体顺序更小），再注册独立规则。
    world.add_data(
        Some(b),
        ReduceBy {
            target_id: b,
            amount: 5,
        },
    );
    register_damage_rule(&mut world, Priority::Modify, HalveRule);
    register_damage_rule(&mut world, Priority::Modify, ReduceByRule);

    world
        .execute(Damage::new(Some(a), b, 10))
        .expect("伤害应正常结算");

    // 共同创建顺序：先减 5 再减半 → (10 - 5) / 2 = 2。
    assert_eq!(
        world.get_player(b).map(|p| p.hp()),
        Some(18),
        "独立规则的排序必须来自共同顺序源，而不是注册表局部下标"
    );
}

// —— F8：多次生命声明在提交时明确拒绝 ——

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

// —— I1：按稳定身份的受控数据更新 ——

/// 有剩余容量的一次性护盾：吸收伤害并扣减容量。
#[derive(Debug)]
struct CapacityShield {
    target_id: PlayerId,
    capacity: u64,
}

#[derive(Debug)]
struct CapacityShieldRule;

impl DamageRule for CapacityShieldRule {
    fn candidates(&self, context: &DamageContext, query: &Query<'_>) -> Vec<Subject> {
        query
            .instances::<CapacityShield>()
            .iter()
            .filter(|(_, _, data)| data.target_id == context.target_id)
            .map(|(id, _, _)| Subject::Instance(*id))
            .collect()
    }
    fn modify(&self, context: &mut DamageContext, subject: Subject, query: &Query<'_>) {
        let Subject::Instance(id) = subject else {
            return;
        };
        let Some(data) = query.data::<CapacityShield>(id) else {
            return;
        };
        // 吸收伤害并声明容量扣减：与最终伤害一起进入受控关联提交。
        let absorbed = data.capacity.min(context.amount);
        context.reduce_to(context.amount - absorbed);
        if absorbed > 0 {
            context.update_buff(
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
    shield: BuffId,
    new_capacity: u64,
}
impl Operation for AbsorbAndUpdate {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        let changes = ChangeSet::new().damage(self.target, 3).update_data(
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
    shield: BuffId,
    observed: Rc<Cell<u64>>,
}
impl System for CapacityObserver {
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
        _fact: &Fact<'_>,
        _subject: Subject,
        query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        if let Some(data) = query.data::<CapacityShield>(self.shield) {
            self.observed.set(data.capacity);
        }
        Ok(vec![])
    }
}

#[test]
fn controlled_update_keeps_identity_and_joins_associated_submission() {
    let mut world = World::new();
    let b = world.add_player(Player::new("B".into(), 20, 0));
    register_damage_rule(&mut world, Priority::Modify, CapacityShieldRule);
    let shield = world.add_data(
        Some(b),
        CapacityShield {
            target_id: b,
            capacity: 10,
        },
    );

    // 关联提交：扣血与容量更新对所有响应同时可见。
    let observed = Rc::new(Cell::new(0u64));
    world.add_system(CapacityObserver {
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
    let data = world.get_data::<CapacityShield>(shield).expect("实例仍在");
    assert_eq!(data.capacity, 7, "更新保留原实例身份");
    assert_eq!(
        world.query().instances::<CapacityShield>().len(),
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
        world.get_data::<CapacityShield>(shield).unwrap().capacity,
        4
    );
}

#[derive(Debug)]
struct ConvenienceUpdate {
    shield: BuffId,
    target: PlayerId,
    capacity: u64,
}
impl Operation for ConvenienceUpdate {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        ctx.update_data(
            self.shield,
            CapacityShield {
                target_id: self.target,
                capacity: self.capacity,
            },
        )?;
        completed()
    }
}

// —— D4 直接场景：System 一次返回的多个操作不被丢弃 ——

#[derive(Debug)]
struct BelovedMark;

#[derive(Debug)]
struct CounterAndHealSystem {
    owner: PlayerId,
    ally: PlayerId,
}
impl System for CounterAndHealSystem {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::RoundStart), Priority::Default)]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &Query<'_>) -> Vec<Subject> {
        matches!(fact.event(), Some(Event::RoundStart { .. }))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![
            Box::new(Damage::new(None, self.owner, 10)),
            Box::new(Heal::new(self.ally, 3)),
        ])
    }
}

#[test]
fn system_returned_operation_list_survives_source_destruction() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let owner = world.add_player(Player::new("Owner".into(), 10, 0));
    let ally = world.add_player(Player::new("Ally".into(), 20, 0));
    world.get_player_mut(ally).unwrap().set_hp(15);
    let mark = world.add_data(Some(owner), BelovedMark);
    world.add_system(CounterAndHealSystem { owner, ally });

    world
        .execute(EmitEvent(Event::RoundStart { round: 1 }))
        .expect("事件应正常分发");

    // 反击先使 owner 死亡并销毁其数据；同一响应已返回的治疗仍按序执行。
    assert!(world.get_player(owner).is_none(), "反击应使 owner 死亡");
    assert!(
        world.get_data::<BelovedMark>(mark).is_none(),
        "owner 数据应随死亡销毁"
    );
    assert_eq!(
        world.get_player(ally).map(|p| p.hp()),
        Some(18),
        "同一响应已返回的操作列表不得因来源销毁被丢弃"
    );
}

// —— F5 相邻场景：技能集合按业务持有者匹配 ——

/// 记录被使用次数的技能。
#[derive(Debug)]
struct TouchAbility(Rc<Cell<usize>>);
impl Ability for TouchAbility {
    fn operation(&self, _source: PlayerId, _state: &GameState) -> Option<Box<dyn Operation>> {
        self.0.set(self.0.get() + 1);
        None
    }
}

#[test]
fn abilities_holder_field_drives_turn_matching() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 10, 0));
    let used = Rc::new(Cell::new(0));
    // owner=A（随 A 销毁），持有者=B：B 的回合应使用这份技能集合。
    world.add_data(
        Some(a),
        Abilities::new(b, vec![Box::new(TouchAbility(used.clone()))]),
    );

    world
        .execute(TurnOperation {
            round: 1,
            player_id: b,
        })
        .expect("B 的回合应正常执行");

    assert_eq!(
        used.get(),
        1,
        "Turn 应按业务持有者字段匹配技能集合，而不是记录 owner"
    );
}
