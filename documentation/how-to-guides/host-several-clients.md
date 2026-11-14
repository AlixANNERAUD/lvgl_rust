# Host several clients

Some hosts share one LVGL between several clients: applications in
sandboxes, WebAssembly modules, scripts, remote sessions. Each client must
reach only its own objects, and when a client goes away its objects must
go too. lvgl-rust leaves that policy to the host: it gives handles, and
the host keeps, for each client, a table of the handles that client may
use. This guide builds that table.

The code below is the `host` module of
[`lvgl-rust/tests/clients.rs`](../../lvgl-rust/tests/clients.rs), which runs
it against LVGL.

## What you need

- lvgl-rust with handles (not views-only mode), configured as in
  [Configure LVGL](configure-lvgl.md).
- An allocator for the tables (`Vec` here). A host without one can use
  fixed arrays: nothing below depends on growing.
- Optionally, the `free-observer` feature, to forget objects LVGL deletes
  on its own (step 5).

## Steps

### 1. Give clients indices, never handles

A client gets a number that means something only in its own table, like
a file descriptor. It never sees a `Handle`: a raw handle names a slot of
the registry shared by every client, so a client that guesses one could
reach another client's object.

```rust
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
}
```

Two clients can hold the same index: it names a different object in each
table, or nothing.

### 2. Resolve every request through the client's table

Look the index up in the table of the client that sent it, then resolve
the handle as usual. A deleted object, or an index from another client,
gives `Error::Dead`.

```rust
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
```

Give each client a root container of its own when it starts (an `Obj` on
the screen, inserted in its table) and create everything else inside it.
Never insert shared objects (a screen, a display layer) in a client's
table: the client could then delete or restyle them.

### 3. Delete a client's objects when it goes away

```rust
impl Client {
    /// Deletes what is left of the client's objects.
    pub fn delete_all(&mut self, lv: &mut Lvgl) {
        for handle in self.objects.drain(..).flatten() {
            // An object deleted with its parent is `Dead`: skipped.
            if let Ok(object) = handle.get_mut(lv) {
                let _ = object.delete();
            }
        }
    }
}
```

The order does not matter: deleting a parent deletes its children, whose
handles then report `Dead`. A deletion the structural checks refuse (a
part of a widget) returns an error and leaves the object to its widget.

### 4. Reuse indices only after removing them

An entry whose object was deleted stays in the table, and resolving it
gives `Error::Dead`, until the host calls `remove`. Reuse an index only
after telling the client it is gone, or an old index of that client
reaches its new object. Handles themselves are never confused: a slot's
generation changes when its object is freed.

### 5. Forget objects LVGL deletes on its own (`free-observer`)

LVGL also deletes objects the client never asked about: children with
their parent, a screen unloaded with auto-deletion. Without help, their
entries stay until the next lookup fails. With the `free-observer`
feature, the host is told about every tracked object LVGL frees:

```toml
[dependencies]
lvgl-rust = { git = "https://github.com/AlixANNERAUD/lvgl_rust", features = ["free-observer"] }
```

The observer is a plain function, called inside LVGL's destructor: it
gets no context, must not call LVGL or lvgl-rust, and cannot enter a
session. Record the handle and process it after the LVGL work:

```rust
use std::sync::Mutex;

/// Objects LVGL freed since the host last looked.
pub static FREED: Mutex<Vec<Handle<Obj>>> = Mutex::new(Vec::new());

pub fn record(freed: lvgl_rust::Freed) {
    // Never held while calling LVGL, so never contended here.
    if let Ok(mut freed_list) = FREED.lock() {
        freed_list.push(freed.handle());
    }
}

impl Client {
    /// Forgets the entries of an object LVGL freed.
    pub fn forget(&mut self, freed: Handle<Obj>) {
        for entry in &mut self.objects {
            if *entry == Some(freed) {
                *entry = None;
            }
        }
    }
}

/// Run after each batch of LVGL work (`timer_handler`, a client request).
pub fn forget_freed(clients: &mut [Client]) {
    let freed = core::mem::take(&mut *FREED.lock().unwrap());
    for handle in freed {
        for client in clients.iter_mut() {
            client.forget(handle);
        }
    }
}
```

Install it once: `lv.set_free_observer(Some(record))`. Without `std`, a
fixed ring of raw handles (`Handle::into_raw`) behind your own lock does
the same. A host with many objects per client can keep a map from handle
to `(client, index)` instead of scanning every table.

## Costs

- The table: one `Option<Handle<Obj>>` per object, 4 bytes, in the host's
  memory. lvgl-rust adds nothing per client.
- `free-observer`: one function pointer, 4 bytes of `.bss` on 32-bit
  targets, and a test in the OBJ_ID free hook. Without it, no event
  callback per object is needed either: stale entries are found by the
  lookup.

## Pitfalls

- **Raw handles are not capabilities.** `Handle::from_raw` accepts any
  number, and a lookup only checks the slot, its generation and the
  class: a guessed number can name any tracked object. Give clients
  indices into their own table.
- **Never call LVGL from the observer.** It runs while LVGL destroys the
  object. Keep the lock it takes free while you call LVGL.
- **Objects the host creates for a client belong in its table,** objects
  it shares with every client do not.

## See also

- [Object lifetime explanation](../explanation/object-lifetime.md)
- [API overview: handles and ownership](../reference/api-overview.md)
- [Features reference](../reference/features.md)
