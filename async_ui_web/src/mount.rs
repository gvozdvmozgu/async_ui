use std::rc::Rc;

use async_ui_core::{
    mount as core_mount,
    vnode::{
        concrete_node::{ConcreteNodeVNode, RefNode},
        GiveVNode,
    },
};
use web_sys::Node;

use crate::{backend::Backend, Render};

pub fn mount_at(render: Render<'static>, node: Node) {
    let fut = GiveVNode::new(
        render,
        Rc::new(ConcreteNodeVNode::new(RefNode::<Backend>::Parent { parent: node }).into()),
    );
    core_mount::<Backend, _>(fut)
}

pub fn mount(render: Render<'static>) {
    let node = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .body()
        .unwrap();
    mount_at(render, node.into())
}
