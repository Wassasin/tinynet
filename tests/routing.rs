use tinynet::routing::{PortId, PortSet};

#[test]
fn port_set_iter() {
    let mut ps = PortSet::empty();
    ps |= PortId::from(2);
    ps |= PortId::from(5);

    let mut iter = ps.into_iter();
    assert_eq!(iter.next(), Some(PortId::from(2)));
    assert_eq!(iter.next(), Some(PortId::from(5)));
    assert_eq!(iter.next(), None);
}
