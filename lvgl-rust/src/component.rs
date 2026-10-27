//! Components: widgets made of other widgets, as plain structs.

use crate::{Error, Handle, Lvgl, Widget};

/// A widget made of other widgets: a plain struct holding its root (a
/// [`Handle`] or an [`Owned`](crate::Owned)) and handles to its parts,
/// built by an associated function in one of two forms:
///
/// - `fn build(parent: &mut ObjMut<'_, impl Widget>) -> Result<Self, Error>`,
///   creating each part through its parent's [`ObjMut`](crate::ObjMut)
///   (creation needs exclusive access);
/// - `fn build(parent: Handle<Obj>, lv: &mut Lvgl) -> Result<Self, Error>`
///   when the caller keeps a handle, resolving `parent` with
///   [`Handle::get_mut`] first.
///
/// Handlers have shared access only: a handler that builds a component
/// posts the build with [`Lvgl::post`], which runs it with exclusive access
/// at the next drain.
///
/// Deleting the root deletes the parts, whose handles then report
/// [`Error::Dead`]. Handlers carry a part's handle with
/// [`Obj::on_with`](crate::Obj::on_with). Nothing is allocated beyond the
/// objects and the registry slots of the handles.
///
/// ```no_run
/// use lvgl_rust::{
///     Component, Error, Handle, Lvgl, Obj, ObjMut, Widget, WidgetExt,
///     enums::EventCode,
///     widgets::{Button, Label},
/// };
///
/// struct Counter {
///     button: Handle<Button>,
///     label: Handle<Label>,
/// }
///
/// impl Counter {
///     fn build(parent: &mut ObjMut<'_, impl Widget>) -> Result<Self, Error> {
///         let mut button = Button::create(parent)?;
///         let label = Label::create(&mut button)?.into_view();
///         label.set_text_i32(0);
///         let label = label.handle()?;
///         let button = button.into_view();
///         button.on_with(EventCode::CLICKED, label, |e, label| {
///             if let Ok(label) = label.get(e.lv()) {
///                 let mut buf = [0u8; 12];
///                 let n: i32 = label
///                     .get_text_into(&mut buf)
///                     .ok()
///                     .flatten()
///                     .and_then(|t| t.to_str().ok()?.parse().ok())
///                     .unwrap_or(0);
///                 label.set_text_i32(n + 1);
///             }
///         })?;
///         Ok(Counter { button: button.handle()?, label })
///     }
/// }
///
/// // From a handler: build later, with exclusive access.
/// fn add_counter(lv: &mut Lvgl, parent: Handle<Obj>) {
///     if let Ok(mut parent) = parent.get_mut(lv) {
///         let _ = Counter::build(&mut parent);
///     }
/// }
///
/// fn install(button: &Obj, parent: Handle<Obj>) -> Result<(), Error> {
///     button.on_with(EventCode::CLICKED, parent, |e, parent| {
///         let _ = e.lv().post(add_counter, parent);
///     })
/// }
///
/// impl Component for Counter {
///     type Root = Button;
///     fn root(&self) -> Handle<Button> {
///         self.button
///     }
/// }
/// ```
pub trait Component: Sized {
    /// The root widget, whose deletion deletes the component.
    type Root: Widget;

    /// The root.
    fn root(&self) -> Handle<Self::Root>;

    /// Deletes the component (its root and so its parts).
    fn delete(self, lv: &mut Lvgl) -> Result<(), Error> {
        self.root().get_mut(lv)?.delete()
    }
}
