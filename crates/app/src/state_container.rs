use async_rwlock::RwLock;
use display::state::AppState;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::lazy_lock::LazyLock;
use embassy_sync::watch::Watch;

pub struct StateStore {
    pub state: RwLock<AppState>,
    pub change_watch: Watch<CriticalSectionRawMutex, AppState, 4>,
}

pub static STATE_STORE: LazyLock<StateStore> = LazyLock::new(|| StateStore {
    state: RwLock::new(AppState::new()),
    change_watch: Watch::new(),
});

pub trait StateStoreExt {
    async fn update<U>(&self, f: U)
    where
        U: FnOnce(&mut AppState),
    {
        self.update_and_trigger(true, f).await;
    }

    async fn update_and_trigger<U>(&self, trigger_update: bool, f: U)
    where
        U: FnOnce(&mut AppState);
}

impl StateStoreExt for LazyLock<StateStore> {
    async fn update_and_trigger<U>(&self, trigger_update: bool, f: U)
    where
        U: FnOnce(&mut AppState),
    {
        let state_store = self.get();
        let new_state = {
            let mut writer = state_store.state.write().await;
            f(&mut writer);
            writer.refresh_updated_counter();
            writer.clone()
        };
        if trigger_update {
            state_store.change_watch.sender().send(new_state);
        }
    }
}
