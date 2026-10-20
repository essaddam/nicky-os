// Fixture fault counter-cases, deliberately outside tests::p1::guard::.
// A passing counter-case proves the fixture failed explicitly AFTER joining.

use super::caller::{Fault, fault_report};

#[tokio::test]
async fn worker_panic_during_drain_is_not_a_business_red() {
    let report = fault_report(Fault::WorkerPanicDuringDrain).await;
    assert!(
        report.assertions_passed(),
        "counter-case body did not finish"
    );
    assert!(
        report.resources_joined(),
        "counter-case resources were not joined"
    );
    assert!(
        report.worker_panic_recorded(),
        "worker panic was not retained"
    );
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || report.enforce()));
    let payload = result.expect_err("enforce accepted a faulty fixture");
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .expect("enforce returned an unexpected panic payload");
    assert!(
        message.contains("CHA_FIXTURE_FAILURE"),
        "unexpected enforce panic: {message}"
    );
}

#[tokio::test]
async fn serve_error_after_assertions_is_not_a_business_red() {
    let report = fault_report(Fault::ServeErrorAfterAssertions).await;
    assert!(
        report.assertions_passed(),
        "counter-case body did not finish"
    );
    assert!(
        report.resources_joined(),
        "counter-case resources were not joined"
    );
    assert!(
        report.serve_error_recorded(),
        "real Link::serve Err was not retained"
    );
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || report.enforce()));
    let payload = result.expect_err("enforce accepted a faulty fixture");
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .expect("enforce returned an unexpected panic payload");
    assert!(
        message.contains("CHA_FIXTURE_FAILURE"),
        "unexpected enforce panic: {message}"
    );
}
