//! A host sharing LVGL between several clients, as in
//! `documentation/how-to-guides/host-several-clients.md`: keep the guide's
//! code in step with the `host` module.
#![cfg(not(lvgl_rust_views_only))]

use lvgl_rust::{Error, Lvgl, LvglAccess, Obj, Session, WidgetExt};
use std::sync::{Mutex, MutexGuard};

/// The code of the guide.
mod host {
    use lvgl_rust::{Error, Handle, Lvgl, Obj, WidgetExt};

    /// What a client holds: an index into its own table, never a handle.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct ClientObject(pub u32);

    /// The objects one client may use.
    #[derive(Default)]
    pub struct Client {
        objects: Vec<Option<Handle<Obj>>>,
    }

    impl Client {
        /// Gives the client an object: returns the index it will use.
        pub fn insert(&mut self, handle: Handle<Obj>) -> ClientObject {
            let index = match self.objects.iter().position(Option::is_none) {
                Some(free) => free,
                None => {
                    self.objects.push(None);
                    self.objects.len() - 1
                }
            };
            self.objects[index] = Some(handle);
            ClientObject(index as u32)
        }

        /// The handle behind an index the client sent. An index the client
        /// was never given fails like a deleted object.
        pub fn handle(&self, object: ClientObject) -> Result<Handle<Obj>, Error> {
            self.objects
                .get(object.0 as usize)
                .copied()
                .flatten()
                .ok_or(Error::Dead)
        }

        /// Forgets an entry, so its index can be given again.
        pub fn remove(&mut self, object: ClientObject) {
            if let Some(entry) = self.objects.get_mut(object.0 as usize) {
                *entry = None;
            }
        }

        /// Deletes what is left of the client's objects.
        pub fn delete_all(&mut self, lv: &mut Lvgl) {
            for handle in self.objects.drain(..).flatten() {
                // An object deleted with its parent is `Dead`: skipped.
                if let Ok(object) = handle.get_mut(lv) {
                    let _ = object.delete();
                }
            }
        }

        /// Forgets the entries of an object LVGL freed.
        #[cfg(lvgl_rust_free_observer)]
        pub fn forget(&mut self, freed: Handle<Obj>) {
            for entry in &mut self.objects {
                if *entry == Some(freed) {
                    *entry = None;
                }
            }
        }
    }

    /// Creates an object inside one the client owns, and gives it to the
    /// client.
    pub fn create_child(
        lv: &mut Lvgl,
        client: &mut Client,
        parent: ClientObject,
    ) -> Result<ClientObject, Error> {
        let mut parent = client.handle(parent)?.get_mut(lv)?;
        let handle = Obj::create(&mut parent)?.handle()?;
        Ok(client.insert(handle))
    }

    /// Objects LVGL freed since the host last looked: the free observer
    /// gets no context and must not call LVGL, so it only records them.
    #[cfg(lvgl_rust_free_observer)]
    pub static FREED: std::sync::Mutex<Vec<Handle<Obj>>> = std::sync::Mutex::new(Vec::new());

    #[cfg(lvgl_rust_free_observer)]
    pub fn record(freed: lvgl_rust::Freed) {
        // Never held while calling LVGL, so never contended here.
        if let Ok(mut freed_list) = FREED.lock() {
            freed_list.push(freed.handle());
        }
    }

    /// Run after each batch of LVGL work, outside any session borrow of
    /// the queue.
    #[cfg(lvgl_rust_free_observer)]
    pub fn forget_freed(clients: &mut [Client]) {
        let freed = core::mem::take(&mut *FREED.lock().unwrap());
        for handle in freed {
            for client in clients.iter_mut() {
                client.forget(handle);
            }
        }
    }
}

use host::{Client, ClientObject, create_child};

static LOCK: Mutex<bool> = Mutex::new(false);

struct Guard(MutexGuard<'static, bool>);

// SAFETY: a guard of the mutex serializing every test's LVGL calls.
unsafe impl LvglAccess for Guard {}

fn guard() -> Guard {
    Guard(LOCK.lock().unwrap_or_else(|p| p.into_inner()))
}

fn session(guard: &mut Guard) -> Session<'_> {
    if !*guard.0 {
        *guard.0 = true;
        let lv = lvgl_rust::init(guard).unwrap();
        // SAFETY: session held.
        unsafe { lvgl_rust::sys::lv_display_create(240, 160) };
        lv
    } else {
        Lvgl::enter(guard).unwrap()
    }
}

/// A root container for a client, on the active screen.
fn root(lv: &mut Lvgl, client: &mut Client) -> ClientObject {
    let mut screen = lv.screen_active_mut().unwrap();
    let handle = Obj::create(&mut screen).unwrap().handle().unwrap();
    client.insert(handle)
}

fn children_of_screen(lv: &Lvgl) -> u32 {
    lv.screen_active().unwrap().get_child_count()
}

#[test]
fn clients_reach_only_their_own_objects() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let (mut a, mut b) = (Client::default(), Client::default());
    let a_root = root(&mut lv, &mut a);
    let b_root = root(&mut lv, &mut b);
    let a_child = create_child(&mut lv, &mut a, a_root).unwrap();
    // Each table has its own indices: the same number names another
    // object, or nothing.
    assert_eq!(a_root, b_root);
    assert_ne!(a.handle(a_root).unwrap(), b.handle(b_root).unwrap());
    assert_eq!(b.handle(a_child).err(), Some(Error::Dead));
    assert_eq!(
        create_child(&mut lv, &mut b, a_child).err(),
        Some(Error::Dead)
    );
    a.delete_all(&mut lv);
    b.delete_all(&mut lv);
}

#[test]
fn a_client_teardown_deletes_its_objects_only() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let before = children_of_screen(&lv);
    let (mut a, mut b) = (Client::default(), Client::default());
    let a_root = root(&mut lv, &mut a);
    let child = create_child(&mut lv, &mut a, a_root).unwrap();
    create_child(&mut lv, &mut a, child).unwrap();
    let b_root = root(&mut lv, &mut b);
    let kept = b.handle(b_root).unwrap();
    a.delete_all(&mut lv);
    assert_eq!(a.handle(a_root).err(), Some(Error::Dead));
    assert!(kept.get(&lv).is_ok());
    assert_eq!(children_of_screen(&lv), before + 1);
    b.delete_all(&mut lv);
    assert_eq!(children_of_screen(&lv), before);
}

#[test]
fn a_removed_entry_is_given_again() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let mut a = Client::default();
    let first = root(&mut lv, &mut a);
    let handle = a.handle(first).unwrap();
    handle.get_mut(&mut lv).unwrap().delete().unwrap();
    assert_eq!(a.handle(first).unwrap().get(&lv).err(), Some(Error::Dead));
    a.remove(first);
    assert_eq!(root(&mut lv, &mut a), first);
    a.delete_all(&mut lv);
}

#[cfg(lvgl_rust_free_observer)]
#[test]
fn the_free_observer_clears_entries_of_objects_deleted_indirectly() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    host::FREED.lock().unwrap().clear();
    lv.set_free_observer(Some(host::record));
    let mut clients = [Client::default(), Client::default()];
    let a_root = root(&mut lv, &mut clients[0]);
    let child = create_child(&mut lv, &mut clients[0], a_root).unwrap();
    let b_root = root(&mut lv, &mut clients[1]);
    // The host deletes the root: the child goes with it, without the
    // client asking.
    let handle = clients[0].handle(a_root).unwrap();
    handle.get_mut(&mut lv).unwrap().delete().unwrap();
    lv.set_free_observer(None);
    drop(lv);
    host::forget_freed(&mut clients);
    assert_eq!(clients[0].handle(a_root).err(), Some(Error::Dead));
    assert_eq!(clients[0].handle(child).err(), Some(Error::Dead));
    assert!(clients[1].handle(b_root).is_ok());
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    clients[1].delete_all(&mut lv);
}
