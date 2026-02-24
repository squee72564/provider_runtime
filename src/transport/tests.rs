use crate::transport::http::{HttpTransport, RetryPolicy};

#[test]
fn test_transport_exports_compile() {
    let policy = RetryPolicy::default();
    let transport = HttpTransport::new(1_000, policy);
    assert!(transport.is_ok());
}
