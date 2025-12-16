pub mod buffer;
pub mod audio_node;

#[cfg(test)]
mod tests {
    use super::audio_node::{Config, FundspElement, FundspSourceElement};
    use crate::databus::slot::StaticSlot;
    use crate::databus::{ConsumerHandle, ProducerHandle};
    use fundsp::prelude::*;
    use rivulets_driver::databus::{Consumer, Producer};
    use rivulets_driver::element::Element;
    use rivulets_driver::format::EmptyFormat;
    use rivulets_driver::port::{InPort, PayloadSize};
    use std::sync::Arc;

    // Helper to create a static slot for f32 data
    type TestSlot = StaticSlot<f32, 128>;

    pub const DEFAULT_PAYLOAD_SIZE: PayloadSize = PayloadSize {
        min: 1,
        preferred: 64,
    };

    #[tokio::test]
    async fn test_math_pipeline() {
        // Pure Math Pipeline
        // Logic: ((1.0 * 2.0) + 3.0) = 5.0 -> clip(0,4) -> 4.0 -> mul(-1.0) -> -4.0
        let node = dc(1.0) >> mul(2.0) >> add(3.0) >> clip_to(0.0, 4.0) >> mul(-1.0);

        let mut element = FundspSourceElement::from_an(node, EmptyFormat, Config::default());

        // Setup output bus
        let slot = Arc::new(TestSlot::new_static());
        let producer = ProducerHandle::new(slot.clone(), DEFAULT_PAYLOAD_SIZE);
        let consumer = ConsumerHandle::new(slot.clone(), DEFAULT_PAYLOAD_SIZE);
        let mut out_port = producer.out_port();

        // Run process once
        element
            .process(&InPort::new_none(), &mut out_port, &mut Default::default())
            .await
            .unwrap();

        // Verify output
        let mut payload = consumer.acquire_read(64).await;
        assert_eq!(payload.len(), 64);
        for &sample in payload.iter() {
            assert_eq!(sample, -4.0);
        }
        
        // Commit consumed data
        payload.commit(64);
    }

    #[tokio::test]
    async fn test_sine_generation() {
        // Sine Wave Generation
        // Logic: sine_hz(440.0) >> clip() >> mul(0.5)
        // Output should be within [-0.5, 0.5]
        let node = sine_hz::<f32>(440.0) >> clip() >> mul(0.5);

        let mut element = FundspSourceElement::from_an(node, EmptyFormat, Config::default());
        element.node.set_sample_rate(48000.0);

        // Setup output bus
        let slot = Arc::new(TestSlot::new_static());
        let producer = ProducerHandle::new(slot.clone(), DEFAULT_PAYLOAD_SIZE);
        let consumer = ConsumerHandle::new(slot.clone(), DEFAULT_PAYLOAD_SIZE);
        let mut out_port = producer.out_port();

        // Run process
        element
            .process(&InPort::new_none(), &mut out_port, &mut Default::default())
            .await
            .unwrap();

        // Verify output range
        let mut payload = consumer.acquire_read(64).await;
        assert_eq!(payload.len(), 64);
        for &sample in payload.iter() {
            assert!(sample >= -0.5 && sample <= 0.5, "Sample {} out of range", sample);
        }
        payload.commit(64);
    }

    #[tokio::test]
    async fn test_processor_chain_with_databus() {
        // Processor and Source connected via Databus

        // Define helper functions for node creation
        let proc = lowpass_hz(500.0, 1.0) >> mul(5.0) >> shape(Tanh(1.0)) >> mul(0.5);

        let source = lfo(|t| {
                if t < 0.5 {
                    0.1
                } else if t < 1.0 {
                    10.0
                } else {
                    0.0
                }
            });

        let sample_rate = 48000.0;

        // Initialize Elements
        let mut source_element =
            FundspSourceElement::from_an(source, EmptyFormat, Config::default());
        source_element.node.set_sample_rate(sample_rate);

        let mut proc_element =
            FundspElement::from_an(proc, EmptyFormat, EmptyFormat, Config::default());
        proc_element.node.set_sample_rate(sample_rate);

        // Initialize Databuses (Slots)
        // Bus 1: Source -> Processor
        let slot1 = Arc::new(TestSlot::new_static());
        let source_prod = ProducerHandle::new(slot1.clone(), DEFAULT_PAYLOAD_SIZE);
        let proc_cons = ConsumerHandle::new(slot1.clone(), DEFAULT_PAYLOAD_SIZE);

        // Bus 2: Processor -> Verifier
        let slot2 = Arc::new(TestSlot::new_static());
        let proc_prod = ProducerHandle::new(slot2.clone(), DEFAULT_PAYLOAD_SIZE);
        let verify_cons = ConsumerHandle::new(slot2.clone(), DEFAULT_PAYLOAD_SIZE);

        // Create Ports
        let mut source_out = source_prod.out_port();
        let in_none = InPort::new_none(); // Source has no input

        let proc_in = proc_cons.in_port();
        let mut proc_out = proc_prod.out_port();

        // Test parameters
        let check_points = vec![
            (0.25, 0.231, 0.001, "Linear region test"),
            (0.75, 0.500, 0.001, "Saturation region test"),
            (1.50, 0.000, 0.00001, "Silence region test"),
        ];

        let total_duration = 2.0;
        let chunk_size = 64;
        let total_samples = (total_duration * sample_rate) as usize;
        let total_chunks = total_samples / chunk_size;

        let mut check_idx = 0;
        let mut processed_samples = 0;

        for _chunk in 0..total_chunks {
            // A. Run Source Element (generates 64 samples to Bus 1)
            source_element
                .process(&in_none, &mut source_out, &mut Default::default())
                .await
                .unwrap();

            // B. Run Processor Element (reads Bus 1, writes Bus 2)
            proc_element
                .process(&proc_in, &mut proc_out, &mut Default::default())
                .await
                .unwrap();

            // C. Verification (reads Bus 2)
            let mut payload = verify_cons.acquire_read(chunk_size).await;
            
            for (i, &sample) in payload.iter().enumerate() {
                let global_sample_idx = processed_samples + i;
                let current_time = global_sample_idx as f64 / sample_rate;

                if check_idx < check_points.len() {
                    let (time, expected, tolerance, desc) = check_points[check_idx];
                    
                    if current_time >= time {
                         // Check only once per checkpoint to avoid spamming assertions
                        // Ensure we haven't drifted too far past the checkpoint (within one chunk)
                        if current_time < time + (chunk_size as f64 / sample_rate) {
                            println!("Checking: {} at {:.4}s (val: {:.4})", desc, current_time, sample);
                            assert!(
                                (sample - expected).abs() < tolerance,
                                "Test failed [{}]: at {:.4}s, expected {}, got {}",
                                desc,
                                current_time,
                                expected,
                                sample
                            );
                            check_idx += 1;
                        }
                    }
                }
            }
            
            payload.commit(chunk_size);
            processed_samples += chunk_size;
        }
    }
}
