//! 针对 2b79eeb 增量审查的回归测试：对应 F1–F4、I1 端到端与 update_data 类型边界。
//! 适配自审查方草案（其语法假设按实际 API 迁移，语义保持一致）。

use duel::core::{
    business::{
        damage::{register_damage_rule, DamageContext, DamageRule},
        revival::{register_revival_system, RevivalData},
    },
    event::{Event, EventType},
    operation::{
        completed, ChangeSet, EmitEvent, ExecutionContext, Operation, OperationError,
        OperationOutcome, RemovePlayerOperation,
    },
    player::Player,
    query::Query,
    system::{Fact, NoticeKind, Priority, Subject, System},
    BuffId, Damage, PlayerId, World,
};

// —— F1：参数修改后重新验证最终目标 ——

#[derive(Debug)]
struct RedirectToken;

/// 把最终目标重定向到已不存在的角色，并声明消费自身。
#[derive(Debug)]
struct RedirectAndConsume {
    token: BuffId,
    destination: PlayerId,
}

impl DamageRule for RedirectAndConsume {
    fn candidates(&self, _damage: &DamageContext, query: &Query<'_>) -> Vec<Subject> {
        if query.data::<RedirectToken>(self.token).is_some() {
            vec![Subject::Instance(self.token)]
        } else {
            Vec::new()
        }
    }

    fn modify(&self, damage: &mut DamageContext, subject: Subject, _query: &Query<'_>) {
        if subject == Subject::Instance(self.token) {
            damage.target_id = self.destination;
            damage.consume_buff(self.token);
        }
    }
}

#[test]
fn damage_revalidates_target_after_parameter_modification() {
    let mut world = World::new();
    let original = world.add_player(Player::new("Original".into(), 20, 0));
    let missing = world.add_player(Player::new("Removed".into(), 20, 0));
    world
        .execute(RemovePlayerOperation(missing))
        .expect("移除应正常结算");
    let token = world.add_data(None, RedirectToken);
    register_damage_rule(
        &mut world,
        Priority::Modify,
        RedirectAndConsume {
            token,
            destination: missing,
        },
    );

    let outcome = world.execute(Damage::new(None, original, 5));
    // 可以选择拒绝非法最终目标，或者正常跳过；均不得消费资源。
    if let Ok((result, _)) = outcome {
        assert_eq!(result, duel::core::OperationResult::Skipped);
    }
    assert_eq!(world.get_player(original).unwrap().hp(), 20);
    assert!(
        world.get_data::<RedirectToken>(token).is_some(),
        "最终目标失效、没有伤害提交时，不应消费以提交成功为前提的资源"
    );
}

// —— F2：候选固定后数据被更新，响应需重新核对当前业务字段 ——

#[derive(Debug)]
struct UpdateRevivalTarget {
    id: BuffId,
    target: PlayerId,
}

impl Operation for UpdateRevivalTarget {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.update_data(
            self.id,
            RevivalData {
                player_id: self.target,
            },
        )?;
        completed()
    }
}

#[derive(Debug)]
struct RetargetBeforeRevival {
    event_player: PlayerId,
    revival: BuffId,
    new_target: PlayerId,
}

impl System for RetargetBeforeRevival {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(
            NoticeKind::Event(EventType::BeforePlayerDeath),
            Priority::Modify,
        )]
    }

    fn candidates(&self, fact: &Fact<'_>, _query: &Query<'_>) -> Vec<Subject> {
        if matches!(fact.event(), Some(Event::BeforePlayerDeath(id)) if *id == self.event_player) {
            vec![Subject::Standalone]
        } else {
            Vec::new()
        }
    }

    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(UpdateRevivalTarget {
            id: self.revival,
            target: self.new_target,
        })])
    }
}

#[test]
fn revival_rechecks_event_target_after_same_id_update() {
    let mut world = World::new();
    let b = world.add_player(Player::new("B".into(), 10, 0));
    let c = world.add_player(Player::new("C".into(), 10, 0));
    // 初始化两名零血角色；不装配默认死亡规则，本例只测试这条通知的匹配。
    world.get_player_mut(b).unwrap().set_hp(0);
    world.get_player_mut(c).unwrap().set_hp(0);
    let revival = world.add_data(None, RevivalData { player_id: b });
    world.add_system(RetargetBeforeRevival {
        event_player: b,
        revival,
        new_target: c,
    });
    register_revival_system(&mut world);

    world
        .execute(EmitEvent(Event::BeforePlayerDeath(b)))
        .expect("事件应正常分发");

    assert_eq!(world.get_player(b).unwrap().hp(), 0);
    assert_eq!(
        world.get_player(c).unwrap().hp(),
        0,
        "B 的死亡前通知不能因为候选数据被更新，而错误地救回 C"
    );
    assert_eq!(
        world
            .get_data::<RevivalData>(revival)
            .map(|data| data.player_id),
        Some(c),
        "更新可以成立，但不再匹配当前事件的机会应保留且不消费"
    );
}

// —— F3：owner 已被删除时，不得新建依赖它的数据 ——

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
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.add_data(Some(self.owner), AttachedData)?;
        completed()
    }
}

#[derive(Debug)]
struct RemoveThenAttach;

impl System for RemoveThenAttach {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::RoundStart), Priority::Default)]
    }

    fn candidates(&self, _fact: &Fact<'_>, query: &Query<'_>) -> Vec<Subject> {
        query
            .instances::<SourceData>()
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
            return Ok(Vec::new());
        };
        let Some(data) = query.data::<SourceData>(id) else {
            return Ok(Vec::new());
        };
        Ok(vec![
            Box::new(RemovePlayerOperation(data.owner)),
            Box::new(AttachAfterOwnerRemoval { owner: data.owner }),
        ])
    }
}

#[test]
fn produced_operation_cannot_attach_data_to_deleted_owner() {
    let mut world = World::new();
    let owner = world.add_player(Player::new("Owner".into(), 10, 0));
    let target = world.add_player(Player::new("Target".into(), 10, 0));
    let source = world.add_data(Some(owner), SourceData { owner });
    world.add_system(RemoveThenAttach);

    // 不限定 Invalid 拒绝或业务无效跳过；只要求不留下有效的孤立依赖数据。
    let _ = world.execute(EmitEvent(Event::RoundStart { round: 1 }));

    assert!(world.get_player(owner).is_none());
    assert!(world.get_player(target).is_some());
    assert!(world.get_data::<SourceData>(source).is_none());
    assert!(
        world.query().instances::<AttachedData>().is_empty(),
        "已死亡 owner 的新依赖不应成为有效实例；D4 不豁免生命周期资格"
    );
}

// —— F4：重复角色移除声明在写入前拒绝 ——

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

// —— I1：容量护盾经真实 Damage 路径扣减容量 ——

#[derive(Debug)]
struct CapacityShield {
    target_id: PlayerId,
    capacity: u64,
}

/// 吸收伤害并声明容量扣减；容量与最终伤害一起进入受控关联提交。
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

#[test]
fn capacity_shield_absorbs_through_real_damage_path() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 6));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    register_damage_rule(&mut world, Priority::Modify, CapacityShieldRule);
    let shield = world.add_data(
        Some(b),
        CapacityShield {
            target_id: b,
            capacity: 10,
        },
    );

    world
        .execute(Damage::new(Some(a), b, 6))
        .expect("第一次伤害");
    assert_eq!(world.get_player(b).unwrap().hp(), 20, "第一击应被完全吸收");
    assert_eq!(
        world.get_data::<CapacityShield>(shield).unwrap().capacity,
        4,
        "容量应随第一次吸收扣减为 4"
    );

    world
        .execute(Damage::new(Some(a), b, 6))
        .expect("第二次伤害");
    assert_eq!(
        world.get_player(b).unwrap().hp(),
        18,
        "第二击应吸收 4 点、穿透 2 点"
    );
    assert_eq!(
        world.get_data::<CapacityShield>(shield).unwrap().capacity,
        0,
        "容量应扣减为 0，实例保留原身份"
    );
}

// —— §5：update_data 的类型边界 ——

#[derive(Debug)]
struct MarkerA;

#[derive(Debug)]
struct MarkerB;

#[derive(Debug)]
struct CrossTypeUpdate {
    id: BuffId,
}
impl Operation for CrossTypeUpdate {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        ctx.update_data(self.id, MarkerB)?;
        completed()
    }
}

#[test]
fn update_data_rejects_cross_type_replacement() {
    let mut world = World::new();
    let id = world.add_data(None, MarkerA);

    let result = world.execute(CrossTypeUpdate { id });

    assert!(
        matches!(result, Err(OperationError::Invalid(_))),
        "跨类型替换应在写入前明确拒绝"
    );
    assert!(
        world.get_data::<MarkerA>(id).is_some(),
        "被拒绝的更新不得改动原实例"
    );
}
