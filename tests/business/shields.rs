//! 护盾消费、容量更新及作用目标与 owner 的分离。

use crate::support::CrossRoleShield;
use duel::core::{
    business::damage::{register_damage_rule, DamageContext, DamageRule},
    install_default_rules,
    player::Player,
    query::Query,
    system::{Priority, Subject},
    AttackOperation, Damage, PlayerId, World,
};

/// 一次性护盾：数据实例 + 业务规则。
/// 规则把伤害草稿降为 0 并声明消耗；消耗由受控提交统一处理。
#[derive(Debug)]
struct AuditShieldData {
    target_id: PlayerId,
}

#[derive(Debug)]
struct AuditShieldRule;

impl DamageRule for AuditShieldRule {
    fn candidates(
        &self,
        context: &DamageContext,
        query: &duel::core::query::Query<'_>,
    ) -> Vec<Subject> {
        query
            .instances::<AuditShieldData>()
            .iter()
            .filter(|(_, _, data)| data.target_id == context.target_id)
            .map(|(id, _, _)| Subject::Instance(*id))
            .collect()
    }

    fn modify(
        &self,
        context: &mut DamageContext,
        subject: Subject,
        query: &duel::core::query::Query<'_>,
    ) {
        let Subject::Instance(id) = subject else {
            return;
        };
        let Some(data) = query.data::<AuditShieldData>(id) else {
            return;
        };
        if data.target_id != context.target_id {
            return;
        }
        context.reduce_to(0);
        context.consume_buff(id);
    }
}

#[test]
fn audit_two_attacks_consume_one_shot_shield_exactly_once() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 3));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    register_damage_rule(&mut world, Priority::Modify, AuditShieldRule);
    let shield = world.add_data(Some(b), AuditShieldData { target_id: b });

    world.execute(AttackOperation::new(a)).expect("第一次攻击");
    assert_eq!(
        world.get_player(b).expect("B 存活").hp(),
        20,
        "护盾应挡下第一次攻击"
    );
    assert!(
        world.get_data::<AuditShieldData>(shield).is_none(),
        "护盾应在被挡下的这次提交中一并消耗"
    );

    // 第二次攻击独立结算，读取护盾消耗后的新状态。
    world.execute(AttackOperation::new(a)).expect("第二次攻击");
    assert_eq!(world.get_player(b).expect("B 存活").hp(), 17);
}

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
