//! 针对 6989c0e 增量审查的回归测试：对应 F1（死亡持有者不得再获得
//! 新的正常行动）与 F2（重定向后按实际受伤者核对外部规则的作用范围）。

use duel::core::{
    business::{
        damage::{register_damage_rule, DamageContext, DamageRule},
        damage_reduction::{add_damage_reduction, register_damage_reduction_rule},
        skills::{Abilities, Ability},
    },
    event::{Event, EventType},
    install_default_rules,
    operation::{Operation, OperationError},
    player::Player,
    query::Query,
    state::GameState,
    system::{Fact, NoticeKind, Priority, Subject, System},
    Damage, Heal, PlayerId, TurnOperation, World,
};

// —— F1：技能数据存活，不代表其持有者仍有正常行动资格 ——

/// 自毁：对使用者自己造成 20 点伤害（20 血角色会死亡）。
#[derive(Debug)]
struct SelfDestruct;

impl Ability for SelfDestruct {
    fn operation(&self, actor: PlayerId, _state: &GameState) -> Option<Box<dyn Operation>> {
        Some(Box::new(Damage::new(None, actor, 20)))
    }
}

/// 治疗队友 3 点。
#[derive(Debug)]
struct HealAlly(PlayerId);

impl Ability for HealAlly {
    fn operation(&self, _actor: PlayerId, _state: &GameState) -> Option<Box<dyn Operation>> {
        Some(Box::new(Heal::new(self.0, 3)))
    }
}

#[test]
fn dead_holder_does_not_get_a_new_normal_action_from_surviving_skill_data() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let sponsor = world.add_player(Player::new("Sponsor".into(), 20, 0));
    let actor = world.add_player(Player::new("Actor".into(), 20, 0));
    let ally = world.add_player(Player::new("Ally".into(), 20, 0));
    world.get_player_mut(ally).unwrap().set_hp(10); // 仅用于测试初始化。

    // 生命周期依赖是 Sponsor；技能的使用者是 Actor。
    let skills = world.add_data(
        Some(sponsor),
        Abilities::new(
            actor,
            vec![Box::new(SelfDestruct), Box::new(HealAlly(ally))],
        ),
    );

    world
        .execute(TurnOperation {
            round: 1,
            player_id: actor,
        })
        .expect("死亡链与回合收尾应正常结算");

    assert!(world.get_player(actor).is_none(), "自毁应使持有者死亡");
    assert!(
        world
            .get_data::<duel::core::business::skills::Abilities>(skills)
            .is_some(),
        "owner 仍存活，技能数据应保留"
    );
    assert!(
        !world.is_end(),
        "Sponsor 与 Ally 仍在，终局判断不得掩盖行动资格问题"
    );
    assert_eq!(
        world.get_player(ally).unwrap().hp(),
        10,
        "持有者死亡后不得再安排新的正常技能：数据存活不等于资格成立"
    );
}

/// 在 Turn 通知的响应中杀死行动者的 System。
#[derive(Debug)]
struct KillOnTurn(PlayerId);

impl System for KillOnTurn {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::Turn), Priority::Default)]
    }

    fn candidates(&self, fact: &Fact<'_>, _query: &Query<'_>) -> Vec<Subject> {
        match fact.event() {
            Some(Event::Turn { player_id, .. }) if *player_id == self.0 => {
                vec![Subject::Standalone]
            }
            _ => Vec::new(),
        }
    }

    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(Damage::new(None, self.0, 20))])
    }
}

#[test]
fn holder_killed_by_turn_notice_does_not_get_a_first_normal_action() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let sponsor = world.add_player(Player::new("Sponsor".into(), 20, 0));
    let actor = world.add_player(Player::new("Actor".into(), 20, 0));
    let ally = world.add_player(Player::new("Ally".into(), 20, 0));
    world.get_player_mut(ally).unwrap().set_hp(10);
    world.add_data(
        Some(sponsor),
        Abilities::new(actor, vec![Box::new(HealAlly(ally))]),
    );
    world.add_system(KillOnTurn(actor));

    world
        .execute(TurnOperation {
            round: 1,
            player_id: actor,
        })
        .expect("回合应在行动者死亡后正常收尾");

    assert!(world.get_player(actor).is_none());
    assert_eq!(
        world.get_player(ally).unwrap().hp(),
        10,
        "Turn 通知前的存活检查在反应后已过期，不得据此安排第一项正常技能"
    );
}

// —— F2：重定向后，防御规则按当前目标核对 ——
//
// 重定向规则先注册：同优先级下依共同顺序源先于减伤候选执行。

/// 把伤害重定向到指定角色的独立规则。
#[derive(Debug)]
struct RedirectTo(PlayerId);

impl DamageRule for RedirectTo {
    fn candidates(&self, _damage: &DamageContext, _query: &Query<'_>) -> Vec<Subject> {
        vec![Subject::Standalone]
    }

    fn modify(&self, damage: &mut DamageContext, _subject: Subject, _query: &Query<'_>) {
        damage.target_id = self.0;
    }
}

#[test]
fn redirect_before_defense_does_not_apply_the_original_targets_reduction() {
    let mut world = World::new();
    let original = world.add_player(Player::new("Original".into(), 20, 0));
    let recipient = world.add_player(Player::new("Recipient".into(), 20, 0));

    register_damage_rule(&mut world, Priority::Modify, RedirectTo(recipient));
    register_damage_reduction_rule(&mut world);
    // 只有原目标有减伤；实际受伤者没有。
    add_damage_reduction(&mut world, original, original, 0.5);

    world
        .execute(Damage::new(None, original, 10))
        .expect("伤害应正常结算");

    assert_eq!(world.get_player(original).unwrap().hp(), 20);
    assert_eq!(
        world.get_player(recipient).unwrap().hp(),
        10,
        "先行的重定向之后，旧目标的减伤候选不得作用于新的实际受伤者"
    );
}

#[test]
fn redirect_before_defense_includes_the_actual_recipients_reduction() {
    let mut world = World::new();
    let original = world.add_player(Player::new("Original".into(), 20, 0));
    let recipient = world.add_player(Player::new("Recipient".into(), 20, 0));

    register_damage_rule(&mut world, Priority::Modify, RedirectTo(recipient));
    register_damage_reduction_rule(&mut world);
    add_damage_reduction(&mut world, recipient, recipient, 0.5);

    world
        .execute(Damage::new(None, original, 10))
        .expect("伤害应正常结算");

    assert_eq!(world.get_player(original).unwrap().hp(), 20);
    assert_eq!(
        world.get_player(recipient).unwrap().hp(),
        15,
        "重定向先于防御选择时，实际受伤者自身的减伤应当参与"
    );
}
