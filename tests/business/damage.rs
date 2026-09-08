//! 伤害参数修改、目标重定向与规则候选排序。

use duel::core::{
    business::{
        damage::{register_damage_rule, DamageDraft, DamageRule},
        damage_reduction::{attach_damage_reduction, register_damage_reduction_rule},
    },
    operation::{HpChange, RemovePlayerOperation},
    player::Player,
    query::Query,
    system::{Priority, ReactionTarget},
    BattleEngine, ComponentId, Damage, PlayerId,
};

#[test]
fn damage_operation_applies_parameter_modifiers_before_submission() {
    let mut world = BattleEngine::new();
    let source = world.add_player(Player::new("A".into(), 10, 1));
    let target = world.add_player(Player::new("B".into(), 20, 1));
    register_damage_reduction_rule(&mut world);
    attach_damage_reduction(&mut world, target, target, 0.25);
    let outcome = world
        .execute(Damage::new(Some(source), target, 8))
        .expect("伤害操作应成功");
    let value = outcome.1.expect("应返回生命结果");
    let change = value.downcast_ref::<HpChange>().expect("类型应匹配");
    assert_eq!((change.old_hp, change.new_hp), (20, 14));
}

#[derive(Debug)]
struct LateShield {
    target_id: PlayerId,
}

#[derive(Debug)]
struct LateShieldRule;

impl DamageRule for LateShieldRule {
    fn candidates(&self, context: &DamageDraft, query: &Query<'_>) -> Vec<ReactionTarget> {
        query
            .components::<LateShield>()
            .iter()
            .filter(|(_, _, data)| data.target_id == context.target_id)
            .map(|(id, _, _)| ReactionTarget::Instance(*id))
            .collect()
    }
    fn modify(&self, context: &mut DamageDraft, subject: ReactionTarget, query: &Query<'_>) {
        let ReactionTarget::Instance(id) = subject else {
            return;
        };
        if query.component::<LateShield>(id).is_none() {
            return;
        }
        context.reduce_to(0);
        context.consume_buff(id);
    }
}

#[test]
fn damage_to_missing_target_does_not_consume_declared_shield() {
    let mut world = BattleEngine::new();
    let a = world.add_player(Player::new("A".into(), 10, 5));
    let b = world.add_player(Player::new("B".into(), 10, 0));
    register_damage_rule(&mut world, Priority::Modify, LateShieldRule);
    let shield = world.attach_component(Some(a), LateShield { target_id: b });

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
        .components::<LateShield>()
        .iter()
        .map(|(id, _, _)| *id)
        .collect();
    assert_eq!(
        shields,
        vec![shield],
        "伤害不成立时，以伤害成功为前提的资源消耗不得提交"
    );
}

/// 减去固定数值的减伤数据。
#[derive(Debug)]
struct ReduceBy {
    target_id: PlayerId,
    amount: u64,
}

#[derive(Debug)]
struct ReduceByRule;

impl DamageRule for ReduceByRule {
    fn candidates(&self, context: &DamageDraft, query: &Query<'_>) -> Vec<ReactionTarget> {
        query
            .components::<ReduceBy>()
            .iter()
            .filter(|(_, _, data)| data.target_id == context.target_id)
            .map(|(id, _, _)| ReactionTarget::Instance(*id))
            .collect()
    }
    fn modify(&self, context: &mut DamageDraft, subject: ReactionTarget, query: &Query<'_>) {
        let ReactionTarget::Instance(id) = subject else {
            return;
        };
        let Some(data) = query.component::<ReduceBy>(id) else {
            return;
        };
        context.reduce_to(context.amount.saturating_sub(data.amount));
    }
}

/// 减半的独立规则（不绑定实例）。
#[derive(Debug)]
struct HalveRule;

impl DamageRule for HalveRule {
    fn candidates(&self, _context: &DamageDraft, _query: &Query<'_>) -> Vec<ReactionTarget> {
        vec![ReactionTarget::Standalone]
    }
    fn modify(&self, context: &mut DamageDraft, _subject: ReactionTarget, _query: &Query<'_>) {
        context.reduce_to(context.amount / 2);
    }
}

#[test]
fn damage_rule_standalone_candidates_share_the_common_order_timeline() {
    let mut world = BattleEngine::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    // 先创建实例（主体顺序更小），再注册独立规则。
    world.attach_component(
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
        world.player(b).map(|p| p.hp()),
        Some(18),
        "独立规则的排序必须来自共同顺序源，而不是注册表局部下标"
    );
}

#[derive(Debug)]
struct RedirectToken;

/// 把最终目标重定向到已不存在的角色，并声明消费自身。
#[derive(Debug)]
struct RedirectAndConsume {
    token: ComponentId,
    destination: PlayerId,
}

impl DamageRule for RedirectAndConsume {
    fn candidates(&self, _damage: &DamageDraft, query: &Query<'_>) -> Vec<ReactionTarget> {
        if query.component::<RedirectToken>(self.token).is_some() {
            vec![ReactionTarget::Instance(self.token)]
        } else {
            Vec::new()
        }
    }

    fn modify(&self, damage: &mut DamageDraft, subject: ReactionTarget, _query: &Query<'_>) {
        if subject == ReactionTarget::Instance(self.token) {
            damage.target_id = self.destination;
            damage.consume_buff(self.token);
        }
    }
}

#[test]
fn damage_revalidates_target_after_parameter_modification() {
    let mut world = BattleEngine::new();
    let original = world.add_player(Player::new("Original".into(), 20, 0));
    let missing = world.add_player(Player::new("Removed".into(), 20, 0));
    world
        .execute(RemovePlayerOperation(missing))
        .expect("移除应正常结算");
    let token = world.attach_component(None, RedirectToken);
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
    assert_eq!(world.player(original).unwrap().hp(), 20);
    assert!(
        world.component::<RedirectToken>(token).is_some(),
        "最终目标失效、没有伤害提交时，不应消费以提交成功为前提的资源"
    );
}

//
// 重定向规则先注册：同优先级下依共同顺序源先于减伤候选执行。

/// 把伤害重定向到指定角色的独立规则。
#[derive(Debug)]
struct RedirectTo(PlayerId);

impl DamageRule for RedirectTo {
    fn candidates(&self, _damage: &DamageDraft, _query: &Query<'_>) -> Vec<ReactionTarget> {
        vec![ReactionTarget::Standalone]
    }

    fn modify(&self, damage: &mut DamageDraft, _subject: ReactionTarget, _query: &Query<'_>) {
        damage.target_id = self.0;
    }
}

#[test]
fn redirect_before_defense_does_not_apply_the_original_targets_reduction() {
    let mut world = BattleEngine::new();
    let original = world.add_player(Player::new("Original".into(), 20, 0));
    let recipient = world.add_player(Player::new("Recipient".into(), 20, 0));

    register_damage_rule(&mut world, Priority::Modify, RedirectTo(recipient));
    register_damage_reduction_rule(&mut world);
    // 只有原目标有减伤；实际受伤者没有。
    attach_damage_reduction(&mut world, original, original, 0.5);

    world
        .execute(Damage::new(None, original, 10))
        .expect("伤害应正常结算");

    assert_eq!(world.player(original).unwrap().hp(), 20);
    assert_eq!(
        world.player(recipient).unwrap().hp(),
        10,
        "先行的重定向之后，旧目标的减伤候选不得作用于新的实际受伤者"
    );
}

#[test]
fn redirect_before_defense_includes_the_actual_recipients_reduction() {
    let mut world = BattleEngine::new();
    let original = world.add_player(Player::new("Original".into(), 20, 0));
    let recipient = world.add_player(Player::new("Recipient".into(), 20, 0));

    register_damage_rule(&mut world, Priority::Modify, RedirectTo(recipient));
    register_damage_reduction_rule(&mut world);
    attach_damage_reduction(&mut world, recipient, recipient, 0.5);

    world
        .execute(Damage::new(None, original, 10))
        .expect("伤害应正常结算");

    assert_eq!(world.player(original).unwrap().hp(), 20);
    assert_eq!(
        world.player(recipient).unwrap().hp(),
        15,
        "重定向先于防御选择时，实际受伤者自身的减伤应当参与"
    );
}
