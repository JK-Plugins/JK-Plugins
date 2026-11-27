//! スレッドで分ける処理の入り口。
//!
//! ふだんは `std::thread::scope` そのもの。スレッドの無い WebAssembly（ブラウザのデモ）では、
//! 同じ書き方のまま、`spawn` した処理をその場で順に実行する。

#[cfg(not(target_arch = "wasm32"))]
pub use std::thread::scope;

#[cfg(target_arch = "wasm32")]
pub struct Scope;

#[cfg(target_arch = "wasm32")]
pub struct Handle<T>(T);

#[cfg(target_arch = "wasm32")]
impl<T> Handle<T> {
    pub fn join(self) -> std::thread::Result<T> {
        Ok(self.0)
    }
}

#[cfg(target_arch = "wasm32")]
impl Scope {
    pub fn spawn<F: FnOnce() -> T, T>(&self, f: F) -> Handle<T> {
        Handle(f())
    }
}

#[cfg(target_arch = "wasm32")]
pub fn scope<F: FnOnce(&Scope) -> T, T>(f: F) -> T {
    f(&Scope)
}
