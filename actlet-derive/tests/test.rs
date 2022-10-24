use actlet::Action;
use actlet::Message;
use actlet::Signal;
use static_assertions::assert_impl_all;
use static_assertions::assert_type_eq_all;

#[derive(Message)]
struct Test0;
assert_impl_all!(Test0: Message, Signal);
assert_type_eq_all!(<Test0 as Message>::Reply, ());

#[derive(Message)]
#[reply("()")]
struct Test1;
assert_impl_all!(Test1: Message, Action);
assert_type_eq_all!(<Test1 as Message>::Reply, ());

#[derive(Message)]
#[reply("usize")]
struct Test2;
assert_impl_all!(Test2: Message, Action);
assert_type_eq_all!(<Test2 as Message>::Reply, usize);

#[derive(Message)]
#[reply("Vec<usize>")]
struct Test3;
assert_impl_all!(Test3: Message, Action);
assert_type_eq_all!(<Test3 as Message>::Reply, Vec<usize>);
