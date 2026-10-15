use turninator::{
    animation::{Timeline, TurntableTiming},
    export::ExportRequest,
    project::OutputPreferences,
};

#[test]
fn selected_rate_changes_frame_count_without_changing_rotation_speed() {
    let timing = TurntableTiming {
        seconds_per_turn: 2.,
        duration_seconds: 6.,
    };
    for (rate, count) in [
        (1, 6),
        (15, 90),
        (24, 144),
        (30, 180),
        (60, 360),
        (100, 600),
    ] {
        assert_eq!(timing.frame_count_at_rate(rate).unwrap(), count);
        assert_eq!(timing.total_turns_at_rate(rate).unwrap(), 3.);
    }
    assert_eq!(
        TurntableTiming {
            duration_seconds: 0.05,
            ..Default::default()
        }
        .frame_count_at_rate(1)
        .unwrap(),
        1
    );
}

#[test]
fn timeline_and_export_request_use_the_selected_sampling_rate() {
    let timeline = Timeline {
        duration_seconds: 2.,
        buildup: true,
        ..Default::default()
    };
    assert_eq!(timeline.frame_count_at_rate(30).unwrap(), 60);
    let frames = timeline.worker_frames_at_rate(30).unwrap();
    assert_eq!(frames.len(), 60);
    assert_eq!(frames[0]["buildup_fraction"], 0.);
    assert_eq!(frames[59]["buildup_fraction"], 1.);
    assert!(timeline.sample_frame_at_rate(60, 30).is_err());
    let mut request = ExportRequest {
        frame_rate: 30,
        timeline: Some(timeline),
        ..Default::default()
    };
    assert_eq!(request.animation_frame_count().unwrap(), 60);
    request.timeline = None;
    request.turntable_timing = Some(TurntableTiming::default());
    assert_eq!(request.animation_frame_count().unwrap(), 120);
}

#[test]
fn invalid_rates_and_oversized_jobs_are_rejected() {
    for rate in [0, 101, u32::MAX] {
        assert!(
            TurntableTiming::default()
                .frame_count_at_rate(rate)
                .is_err()
        );
        assert!(Timeline::default().frame_count_at_rate(rate).is_err());
        assert!(
            ExportRequest {
                frame_rate: rate,
                ..Default::default()
            }
            .animation_frame_count()
            .is_err()
        );
    }
    let timing = TurntableTiming {
        duration_seconds: 500.,
        ..Default::default()
    };
    assert_eq!(timing.frame_count_at_rate(20).unwrap(), 10000);
    assert!(timing.frame_count_at_rate(100).is_err());
}

#[test]
fn projects_and_legacy_requests_keep_default_twenty_fps() {
    let options: OutputPreferences = serde_json::from_str("{}").unwrap();
    assert_eq!(options.frame_rate, 20);
    let options: OutputPreferences =
        serde_json::from_value(serde_json::json!({"frame_rate":24})).unwrap();
    let saved = serde_json::to_value(options).unwrap();
    assert_eq!(saved["frame_rate"], 24);
    let mut request = serde_json::to_value(ExportRequest::default()).unwrap();
    request.as_object_mut().unwrap().remove("frame_rate");
    let request: ExportRequest = serde_json::from_value(request).unwrap();
    assert_eq!(request.frame_rate, 20);
}
