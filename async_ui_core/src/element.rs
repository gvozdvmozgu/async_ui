use std::{
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use async_executor::Task;
use pin_cell::{PinCell, PinMut};
use pin_weak::rc::PinWeak;

use crate::{drop_check::PropagateDropScope, executor::LOCAL_EXECUTOR};

use super::{backend::Backend, control::Control};

pin_project_lite::pin_project! {
    struct ElementInner<B: Backend, F: Future<Output = ()>> {
        control: Control<B>,
        task: Option<Task<()>>,
        #[pin]
        future: Option<F>
    }
}
pub struct Element<'e, B: Backend>(Pin<Rc<dyn ElementTrait<B> + 'e>>);
struct WeakElement<B: Backend>(PinWeak<dyn ElementTrait<B> + 'static>);

trait ElementTrait<B: Backend> {
    fn mount(self: Pin<Rc<Self>>, control: Control<B>);
    fn update(self: Pin<&Self>, cx: &mut Context<'_>);
    fn unmount(self: Pin<Rc<Self>>);
}

impl<'e, B: Backend> Element<'e, B> {
    pub(crate) unsafe fn mount(&mut self, control: Control<B>) {
        self.0.clone().mount(control)
    }
}

impl<'e, B: Backend> Drop for Element<'e, B> {
    fn drop(&mut self) {
        self.0.clone().unmount();
    }
}

impl<B: Backend, F: Future<Output = ()>> ElementTrait<B> for PinCell<ElementInner<B, F>> {
    fn mount(self: Pin<Rc<Self>>, control: Control<B>) {
        let mut inner = self.as_ref().borrow_mut();
        let weakened = PinWeak::downgrade((self.clone() as Pin<Rc<dyn ElementTrait<B>>>).clone());
        let this = PinMut::as_mut(&mut inner).project();
        *this.control = control;
        let lifetime_extended = unsafe {
            std::mem::transmute::<
                PinWeak<dyn ElementTrait<B>>,
                PinWeak<dyn ElementTrait<B> + 'static>,
            >(weakened)
        };
        let wrapped = WeakElement(lifetime_extended);
        let task = LOCAL_EXECUTOR.with(|exe| exe.spawn(wrapped));
        *this.task = Some(task);
    }
    fn update(self: Pin<&Self>, cx: &mut Context<'_>) {
        let mut inner = self.borrow_mut();
        let this = PinMut::as_mut(&mut inner).project();
        let fut = this.future.as_pin_mut().expect("polled after unmount");
        let _ = B::get_tls().set(this.control, || fut.poll(cx));
    }
    fn unmount(self: Pin<Rc<Self>>) {
        let mut inner = self.as_ref().borrow_mut();
        let mut this = PinMut::as_mut(&mut inner).project();
        *this.task = None;
        this.future.set(None);
    }
}

impl<B: Backend> Future for WeakElement<B> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let upgraded = self.0.upgrade().expect("polled after unmount");
        let derefed = Pin::as_ref(&upgraded);
        derefed.update(cx);
        Poll::Pending
    }
}

impl<'e, B: Backend, F: Future<Output = ()> + 'e> From<F> for Element<'e, B> {
    fn from(fut: F) -> Self {
        let fut = PropagateDropScope::new(fut);
        let ptr = Rc::pin(PinCell::new(ElementInner {
            control: B::get_dummy_control(),
            future: Some(fut),
            task: None,
        }));
        Self(ptr as _)
    }
}
