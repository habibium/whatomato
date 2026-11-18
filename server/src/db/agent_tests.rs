use sqlx::PgPool;
use uuid::Uuid;

use super::{
    agent_activity::{self, ActivityChat, NewActivity},
    agent_approvals::{self, Became, NewApproval},
    agent_connections::{self, Connection, Credential, MAX_PER_USER, NewConnection, Revoked},
    deliveries::{self, Filter, NewDelivery},
};
use crate::{
    agents::{ApprovalKind, ApprovalStatus, Grant, Outcome, Preset, key::AccessKey},
    scheduler::RecipientType,
};

const ADA: &str = "8801700000001";
const BOB: &str = "8801700000002";

async fn user(pool: &PgPool) -> Uuid {
    sqlx::query_scalar!(
        "INSERT INTO users (email, password) VALUES ($1, 'unused') RETURNING id",
        format!("{}@example.com", Uuid::new_v4()),
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn try_connect(pool: &PgPool, owner: Uuid) -> Option<Connection> {
    let key = AccessKey::generate().unwrap();
    agent_connections::create(
        pool,
        NewConnection {
            user_id: owner,
            name: "Assistant",
            credential: Credential::Key {
                hash: &key.hash,
                hint: &key.hint,
            },
            grant: &Preset::AskBeforeSending.grant(),
            expires_at: None,
        },
    )
    .await
    .unwrap()
}

async fn connect(pool: &PgPool, owner: Uuid) -> Connection {
    try_connect(pool, owner).await.unwrap()
}

async fn deliver(pool: &PgPool, owner: Uuid, connection: Option<Uuid>, to: &str) -> Uuid {
    deliveries::create(
        pool,
        NewDelivery {
            reply_to: None,
            mentions: &[],
            user_id: owner,
            scheduled_message_id: None,
            connection_id: connection,
            recipient_type: RecipientType::Contact,
            recipient: to,
            recipient_name: None,
            body: "hello",
        },
    )
    .await
    .unwrap()
}

async fn ask(pool: &PgPool, connection: Uuid, kind: ApprovalKind, to: &str) -> Uuid {
    let id = Uuid::new_v4();
    agent_approvals::create(
        pool,
        NewApproval {
            id,
            connection_id: connection,
            kind,
            recipient_type: RecipientType::Contact,
            recipient: to,
            recipient_name: Some("Ada"),
            payload: b"sealed",
        },
    )
    .await
    .unwrap()
    .unwrap();
    id
}

#[sqlx::test]
async fn a_user_has_at_most_20_connections(pool: PgPool) {
    let owner = user(&pool).await;
    for _ in 0..MAX_PER_USER {
        connect(&pool, owner).await;
    }
    assert!(try_connect(&pool, owner).await.is_none());
    assert!(try_connect(&pool, user(&pool).await).await.is_some());
}

#[sqlx::test]
async fn two_creates_at_the_cap_cannot_both_pass(pool: PgPool) {
    let owner = user(&pool).await;
    for _ in 0..MAX_PER_USER - 1 {
        connect(&pool, owner).await;
    }
    let (first, second) = tokio::join!(try_connect(&pool, owner), try_connect(&pool, owner));
    assert_eq!(
        usize::from(first.is_some()) + usize::from(second.is_some()),
        1
    );
    assert_eq!(
        agent_connections::list(&pool, owner).await.unwrap().len(),
        20
    );
}

#[sqlx::test]
async fn the_stored_grant_reads_back(pool: PgPool) {
    let owner = user(&pool).await;
    let connection = connect(&pool, owner).await;
    assert_eq!(
        Grant::parse(&connection.grant),
        Ok(Preset::AskBeforeSending.grant())
    );

    let full = Preset::FullAccess.grant();
    let changed = agent_connections::set_grant(&pool, connection.id, owner, &full)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(Grant::parse(&changed.grant), Ok(full));

    let stranger = user(&pool).await;
    assert!(
        agent_connections::set_grant(&pool, connection.id, stranger, &Preset::ViewOnly.grant())
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test]
async fn a_key_finds_its_connection_and_says_if_it_is_paused_or_expired(pool: PgPool) {
    let owner = user(&pool).await;
    let key = AccessKey::generate().unwrap();
    let connection = agent_connections::create(
        &pool,
        NewConnection {
            user_id: owner,
            name: "Assistant",
            credential: Credential::Key {
                hash: &key.hash,
                hint: &key.hint,
            },
            grant: &Preset::ViewOnly.grant(),
            expires_at: Some(chrono::Utc::now() + chrono::TimeDelta::days(1)),
        },
    )
    .await
    .unwrap()
    .unwrap();

    let found = agent_connections::find_by_key(&pool, &key.hash)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.connection.id, connection.id);
    assert!(!found.paused && !found.expired);

    assert_eq!(agent_connections::pause_all(&pool, owner).await.unwrap(), 1);
    assert_eq!(agent_connections::pause_all(&pool, owner).await.unwrap(), 0);
    let found = agent_connections::find_by_key(&pool, &key.hash)
        .await
        .unwrap()
        .unwrap();
    assert!(found.paused);
    agent_connections::resume(&pool, connection.id, owner)
        .await
        .unwrap()
        .unwrap();

    sqlx::query!(
        "UPDATE agent_connections SET expires_at = now() WHERE id = $1",
        connection.id
    )
    .execute(&pool)
    .await
    .unwrap();
    let found = agent_connections::find_by_key(&pool, &key.hash)
        .await
        .unwrap()
        .unwrap();
    assert!(!found.paused && found.expired);

    let rotated = AccessKey::generate().unwrap();
    agent_connections::rotate_key(&pool, connection.id, owner, &rotated.hash, &rotated.hint)
        .await
        .unwrap()
        .unwrap();
    assert!(
        agent_connections::find_by_key(&pool, &key.hash)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        agent_connections::find_by_key(&pool, &rotated.hash)
            .await
            .unwrap()
            .is_some()
    );
}

#[sqlx::test]
async fn use_is_recorded_at_most_once_a_minute(pool: PgPool) {
    let owner = user(&pool).await;
    let connection = connect(&pool, owner).await;
    agent_connections::touch(&pool, connection.id)
        .await
        .unwrap();
    let first = agent_connections::find(&pool, connection.id, owner)
        .await
        .unwrap()
        .unwrap()
        .last_used_at
        .unwrap();
    agent_connections::touch(&pool, connection.id)
        .await
        .unwrap();
    let second = agent_connections::find(&pool, connection.id, owner)
        .await
        .unwrap()
        .unwrap()
        .last_used_at;
    assert_eq!(second, Some(first));

    sqlx::query!(
        "UPDATE agent_connections SET last_used_at = now() - interval '61 seconds' WHERE id = $1",
        connection.id
    )
    .execute(&pool)
    .await
    .unwrap();
    agent_connections::touch(&pool, connection.id)
        .await
        .unwrap();
    let third = agent_connections::find(&pool, connection.id, owner)
        .await
        .unwrap()
        .unwrap()
        .last_used_at
        .unwrap();
    assert!(third > first);
}

#[sqlx::test]
async fn counts_cover_its_deliveries_and_waiting_sends_of_the_last_day(pool: PgPool) {
    let owner = user(&pool).await;
    let agent = connect(&pool, owner).await.id;
    let other = connect(&pool, owner).await.id;

    deliver(&pool, owner, Some(agent), ADA).await;
    deliver(&pool, owner, Some(agent), BOB).await;
    let old = deliver(&pool, owner, Some(agent), ADA).await;
    sqlx::query!(
        "UPDATE deliveries SET created_at = now() - interval '25 hours' WHERE id = $1",
        old
    )
    .execute(&pool)
    .await
    .unwrap();
    deliver(&pool, owner, Some(other), ADA).await;
    deliver(&pool, owner, None, ADA).await;

    ask(&pool, agent, ApprovalKind::Send, ADA).await;
    ask(&pool, agent, ApprovalKind::ScheduleCreate, ADA).await;
    let rejected = ask(&pool, agent, ApprovalKind::Send, ADA).await;
    agent_approvals::reject(&pool, rejected, owner)
        .await
        .unwrap()
        .unwrap();
    let expired = ask(&pool, agent, ApprovalKind::Send, ADA).await;
    sqlx::query!(
        "UPDATE agent_approvals SET expires_at = now() WHERE id = $1",
        expired
    )
    .execute(&pool)
    .await
    .unwrap();

    let counts = agent_connections::counts(&pool, agent, RecipientType::Contact, ADA)
        .await
        .unwrap();
    assert_eq!(counts.chat_sends, 2, "a delivery and a waiting send");
    assert_eq!(counts.connection_sends, 3);
    assert_eq!(counts.waiting_approvals, 2, "a send and a schedule");

    let to_group = agent_connections::counts(&pool, agent, RecipientType::Group, ADA)
        .await
        .unwrap();
    assert_eq!(to_group.chat_sends, 0);

    assert_eq!(
        agent_approvals::count_waiting(&pool, owner).await.unwrap(),
        2
    );
}

#[sqlx::test]
async fn an_approval_is_decided_once(pool: PgPool) {
    let owner = user(&pool).await;
    let agent = connect(&pool, owner).await.id;
    let id = ask(&pool, agent, ApprovalKind::Send, ADA).await;
    let delivery = deliver(&pool, owner, Some(agent), ADA).await;
    let became = Became {
        delivery_id: Some(delivery),
        scheduled_message_id: None,
    };

    let (first, second) = tokio::join!(
        agent_approvals::approve(&pool, id, owner, became),
        agent_approvals::approve(&pool, id, owner, became),
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    assert_eq!(
        usize::from(first.is_some()) + usize::from(second.is_some()),
        1
    );
    assert!(
        agent_approvals::reject(&pool, id, owner)
            .await
            .unwrap()
            .is_none()
    );

    let approved = agent_approvals::find(&pool, id, owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approved.status, ApprovalStatus::Approved);
    assert_eq!(approved.delivery_id, Some(delivery));
    assert!(approved.decided_at.is_some());

    let stranger = user(&pool).await;
    let other = ask(&pool, agent, ApprovalKind::Send, ADA).await;
    assert!(
        agent_approvals::approve(&pool, other, stranger, became)
            .await
            .unwrap()
            .is_none()
    );
    let elsewhere = connect(&pool, owner).await.id;
    assert!(
        agent_approvals::find_for_connection(&pool, other, elsewhere)
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test]
async fn an_overdue_approval_reads_expired_and_cannot_be_approved(pool: PgPool) {
    let owner = user(&pool).await;
    let agent = connect(&pool, owner).await.id;
    let overdue = ask(&pool, agent, ApprovalKind::Send, ADA).await;
    let waiting = ask(&pool, agent, ApprovalKind::Send, BOB).await;
    sqlx::query!(
        "UPDATE agent_approvals SET expires_at = now(), \
         created_at = now() + interval '1 minute' WHERE id = $1",
        overdue
    )
    .execute(&pool)
    .await
    .unwrap();

    let found = agent_approvals::find_for_connection(&pool, overdue, agent)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.status, ApprovalStatus::Expired);
    assert!(
        agent_approvals::approve(&pool, overdue, owner, Became::default())
            .await
            .unwrap()
            .is_none()
    );

    // Waiting first, although the overdue one is newer
    let listed = agent_approvals::list(&pool, owner, None, 10, 0)
        .await
        .unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|listed| listed.approval.id)
            .collect::<Vec<_>>(),
        [waiting, overdue]
    );
    assert_eq!(
        agent_approvals::list_for_connection(&pool, agent, 10, 0)
            .await
            .unwrap()
            .len(),
        2
    );

    assert_eq!(agent_approvals::expire_overdue(&pool).await.unwrap(), 1);
    let swept = agent_approvals::find(&pool, overdue, owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(swept.status, ApprovalStatus::Expired);
    assert!(swept.decided_at.is_some());
}

async fn schedule(pool: &PgPool, owner: Uuid, connection: Option<Uuid>) -> Uuid {
    sqlx::query_scalar!(
        "INSERT INTO scheduled_messages (user_id, connection_id, recipient_type, recipient, body, \
         schedule_kind, run_at, timezone) \
         VALUES ($1, $2, 'contact', '8801700000001', 'hello', 'once', now() + interval '1 day', \
                 'UTC') RETURNING id",
        owner,
        connection,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn activity(pool: &PgPool, owner: Uuid, connection: &Connection) {
    agent_activity::insert(
        pool,
        NewActivity {
            user_id: owner,
            connection_id: connection.id,
            connection_name: &connection.name,
            tool: "send_message",
            chat: Some(ActivityChat {
                recipient_type: RecipientType::Contact,
                recipient: ADA,
                recipient_name: Some("Ada"),
            }),
            outcome: Outcome::Asked,
            reason: None,
        },
    )
    .await
    .unwrap();
}

#[sqlx::test]
async fn revoking_keeps_history_and_optionally_its_schedules(pool: PgPool) {
    let owner = user(&pool).await;
    let agent = connect(&pool, owner).await;
    let mine = schedule(&pool, owner, None).await;
    let its = schedule(&pool, owner, Some(agent.id)).await;
    let delivery = deliver(&pool, owner, Some(agent.id), ADA).await;
    ask(&pool, agent.id, ApprovalKind::Send, ADA).await;
    activity(&pool, owner, &agent).await;

    let only_its =
        super::scheduled_messages::list(&pool, owner, Some(agent.id), super::Chats::default())
            .await
            .unwrap();
    assert_eq!(
        only_its
            .iter()
            .map(|message| message.id)
            .collect::<Vec<_>>(),
        [its]
    );
    let filter = Filter {
        connection_id: Some(agent.id),
        ..Filter::default()
    };
    assert_eq!(deliveries::count(&pool, owner, filter).await.unwrap(), 1);

    assert_eq!(
        agent_connections::delete(&pool, agent.id, owner, false)
            .await
            .unwrap(),
        Some(Revoked {
            schedules_deleted: 0,
            schedules_kept: 1
        })
    );
    assert!(
        agent_connections::delete(&pool, agent.id, owner, false)
            .await
            .unwrap()
            .is_none()
    );

    let kept = super::scheduled_messages::find(&pool, its, owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept.connection_id, None);
    let delivered = deliveries::page(&pool, owner, Filter::default(), 10, 0)
        .await
        .unwrap();
    assert_eq!(delivered[0].id, delivery);
    assert_eq!(delivered[0].connection_id, None);
    assert_eq!(
        agent_approvals::list(&pool, owner, None, 10, 0)
            .await
            .unwrap()
            .len(),
        0
    );
    let logged = agent_activity::page(&pool, owner, None, 10, 0)
        .await
        .unwrap();
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].connection_id, None);
    assert_eq!(logged[0].connection_name, "Assistant");
    assert!(
        super::scheduled_messages::find(&pool, mine, owner)
            .await
            .unwrap()
            .is_some()
    );
}

#[sqlx::test]
async fn revoking_with_its_schedules_deletes_only_those(pool: PgPool) {
    let owner = user(&pool).await;
    let agent = connect(&pool, owner).await;
    let mine = schedule(&pool, owner, None).await;
    let its = schedule(&pool, owner, Some(agent.id)).await;

    // Another user can't revoke it
    let stranger = user(&pool).await;
    assert!(
        agent_connections::delete(&pool, agent.id, stranger, true)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        super::scheduled_messages::find(&pool, its, owner)
            .await
            .unwrap()
            .is_some()
    );

    assert_eq!(
        agent_connections::delete(&pool, agent.id, owner, true)
            .await
            .unwrap(),
        Some(Revoked {
            schedules_deleted: 1,
            schedules_kept: 0
        })
    );
    assert!(
        super::scheduled_messages::find(&pool, its, owner)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        super::scheduled_messages::find(&pool, mine, owner)
            .await
            .unwrap()
            .is_some()
    );
}

#[sqlx::test]
async fn activity_pages_by_connection_and_ages_out(pool: PgPool) {
    let owner = user(&pool).await;
    let first = connect(&pool, owner).await;
    let second = connect(&pool, owner).await;
    activity(&pool, owner, &first).await;
    activity(&pool, owner, &first).await;
    activity(&pool, owner, &second).await;

    assert_eq!(agent_activity::count(&pool, owner, None).await.unwrap(), 3);
    assert_eq!(
        agent_activity::count(&pool, owner, Some(first.id))
            .await
            .unwrap(),
        2
    );
    let page = agent_activity::page(&pool, owner, Some(second.id), 10, 0)
        .await
        .unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].outcome, Outcome::Asked);
    assert_eq!(page[0].recipient.as_deref(), Some(ADA));

    sqlx::query!(
        "UPDATE agent_activity SET created_at = now() - interval '91 days' WHERE connection_id = $1",
        first.id
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(agent_activity::delete_old(&pool).await.unwrap(), 2);
    assert_eq!(agent_activity::count(&pool, owner, None).await.unwrap(), 1);
}
