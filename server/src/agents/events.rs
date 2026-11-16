//! Tells open pages that something changed, so they needn't wait for their next poll

use futures_util::{Stream, stream};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
pub struct Events {
    /// The user whose approvals changed
    approvals: broadcast::Sender<Uuid>,
    closed: CancellationToken,
}

impl Default for Events {
    fn default() -> Self {
        Self {
            approvals: broadcast::channel(256).0,
            closed: CancellationToken::new(),
        }
    }
}

impl Events {
    /// A request of `user_id` began to wait, or stopped waiting
    pub fn approvals_changed(&self, user_id: Uuid) {
        // An error only means no page is listening
        let _ = self.approvals.send(user_id);
    }

    /// Ends every stream. An open page would otherwise hold a shutdown up for as long as it stays
    /// open.
    pub fn close(&self) {
        self.closed.cancel();
    }

    /// Yields each time the approvals of `user_id` change, until [`Self::close`]
    pub fn approvals_of(&self, user_id: Uuid) -> impl Stream<Item = ()> + Send + use<> {
        let state = (self.approvals.subscribe(), self.closed.clone());
        stream::unfold(state, move |(mut changes, closed)| async move {
            loop {
                tokio::select! {
                    () = closed.cancelled() => return None,
                    changed = changes.recv() => match changed {
                        Ok(id) if id == user_id => return Some(((), (changes, closed))),
                        Ok(_) => {}
                        // Some were missed: say so, and the page reads what is there now
                        Err(RecvError::Lagged(_)) => return Some(((), (changes, closed))),
                        Err(RecvError::Closed) => return None,
                    },
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures_util::StreamExt as _;
    use tokio::time::timeout;

    use super::*;

    #[tokio::test]
    async fn a_page_hears_of_its_own_users_changes_until_the_server_closes() {
        let events = Events::default();
        let (ada, grace) = (Uuid::new_v4(), Uuid::new_v4());
        let mut adas = Box::pin(events.approvals_of(ada));

        events.approvals_changed(grace);
        events.approvals_changed(ada);
        assert_eq!(adas.next().await, Some(()));
        // Grace's change was passed over, and nothing else is waiting
        let quiet = timeout(Duration::from_millis(50), adas.next()).await;
        assert!(quiet.is_err());

        events.close();
        assert_eq!(adas.next().await, None);
    }
}
