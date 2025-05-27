use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, PartialEq)]
pub enum StepDirective {
    // The caller must perform work as usual for this step.
    Proceed,
    // The caller should skip this step.
    Skip,
}

// The multi-device synchroniser can be used to synchronise a single test
// running across multiple devices.
// Each device's test needs its own DeviceSynchroniser linked to this
// Synchroniser. Only one DeviceSynchroniser can exist per device_id and
// Synchroniser.
pub struct Synchroniser {
    counters: Vec<AtomicU64>,
    // per_device_leases is used to prevent more than one DeviceSynchroniser per
    // device. This is probably entirely overkill, but it wasn't hard to
    // implement so why not?
    // This could be a Vec<Arc<Mutex<bool>>> (which would reduce contention when
    // checking & obtaining leases), but that adds about 5 lines of loops and we
    // don't need the performance benefit.
    per_device_leases: Arc<Mutex<Vec<bool>>>,
}

impl Synchroniser {
    pub fn new(device_count: usize) -> Arc<Synchroniser> {
        let mut counters = Vec::with_capacity(device_count);
        for _ in 0..device_count {
            counters.push(AtomicU64::new(0));
        }
        Arc::new(Synchroniser {
            counters: counters,
            per_device_leases: Arc::new(Mutex::new(vec![false; device_count])),
        })
    }
}

pub struct DeviceSynchroniser {
    device_id: usize,
    synchroniser: Arc<Synchroniser>,
}

impl DeviceSynchroniser {
    fn new(
        synchroniser: &Arc<Synchroniser>,
        device_id: usize,
    ) -> Result<DeviceSynchroniser, String> {
        let mut leases = synchroniser.per_device_leases.lock().unwrap();
        if device_id >= leases.len() {
            return Err("Cannot obtain lease for out of bounds device {device_id}".to_string());
        }

        if leases[device_id] {
            return Err("Lease for device {device_id} is already held".to_string());
        }

        leases[device_id] = true;
        Ok(DeviceSynchroniser {
            device_id,
            synchroniser: synchroniser.clone(),
        })
    }

    pub fn try_step(&self) -> StepDirective {
        let mut min = u64::MAX;
        for counter in &self.synchroniser.counters {
            let val = counter.load(Ordering::Relaxed);
            min = std::cmp::min(min, val);
        }

        let this = &self.synchroniser.counters[self.device_id];
        match this.fetch_update(Ordering::Acquire, Ordering::Relaxed, |x| {
            if x <= min {
                Some(x + 1)
            } else {
                None
            }
        }) {
            Ok(_) => StepDirective::Proceed,
            Err(_) => StepDirective::Skip,
        }
    }
}

impl Drop for DeviceSynchroniser {
    fn drop(&mut self) {
        let mut leases = self.synchroniser.per_device_leases.lock().unwrap();
        leases[self.device_id] = false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_device() {
        let synchroniser = Synchroniser::new(1);
        assert!(
            DeviceSynchroniser::new(&synchroniser, 1).is_err(),
            "invalid device id cannot be leased"
        );
        {
            let dev0 = DeviceSynchroniser::new(&synchroniser, 0).unwrap();
            assert!(
                DeviceSynchroniser::new(&synchroniser, 0).is_err(),
                "device cannot be leased more than once"
            );
            for _ in 0..100 {
                assert_eq!(StepDirective::Proceed, dev0.try_step())
            }
        }
        // Check that leases are correctly returned. (I don't expect this to actually
        // be relevant, ever.)
        {
            let dev0 = DeviceSynchroniser::new(&synchroniser, 0).unwrap();
            assert!(
                DeviceSynchroniser::new(&synchroniser, 0).is_err(),
                "device cannot be leased more than once"
            );
            for _ in 0..100 {
                assert_eq!(StepDirective::Proceed, dev0.try_step())
            }
        }
    }

    #[test]
    fn test_two_devices() {
        let synchroniser = Synchroniser::new(2);
        assert!(
            DeviceSynchroniser::new(&synchroniser, 2).is_err(),
            "invalid device id cannot be leased"
        );
        let dev0 = DeviceSynchroniser::new(&synchroniser, 0).unwrap();
        assert!(
            DeviceSynchroniser::new(&synchroniser, 0).is_err(),
            "device cannot be leased more than once"
        );

        assert_eq!(StepDirective::Proceed, dev0.try_step());
        for _ in 0..30 {
            assert_eq!(StepDirective::Skip, dev0.try_step())
        }
        let dev1 = DeviceSynchroniser::new(&synchroniser, 1).unwrap();
        assert_eq!(StepDirective::Proceed, dev1.try_step());

        for _ in 0..100 {
            assert_eq!(StepDirective::Proceed, dev0.try_step());
            assert_eq!(StepDirective::Proceed, dev1.try_step());
        }
        for _ in 0..100 {
            assert_eq!(StepDirective::Proceed, dev1.try_step());
            assert_eq!(StepDirective::Proceed, dev0.try_step());
        }

        assert_eq!(StepDirective::Proceed, dev1.try_step());
        for _ in 0..30 {
            assert_eq!(StepDirective::Skip, dev1.try_step())
        }
        assert_eq!(StepDirective::Proceed, dev0.try_step());
        assert_eq!(StepDirective::Proceed, dev0.try_step());
    }

    #[test]
    fn test_three_devices() {
        let synchroniser = Synchroniser::new(3);
        let dev2 = DeviceSynchroniser::new(&synchroniser, 2).unwrap();
        let dev1 = DeviceSynchroniser::new(&synchroniser, 1).unwrap();
        let dev0 = DeviceSynchroniser::new(&synchroniser, 0).unwrap();

        for _ in 0..30 {
            assert_eq!(StepDirective::Proceed, dev2.try_step());
            assert_eq!(StepDirective::Proceed, dev1.try_step());
            assert_eq!(StepDirective::Proceed, dev0.try_step());
        }
        for _ in 0..30 {
            assert_eq!(StepDirective::Proceed, dev1.try_step());
            assert_eq!(StepDirective::Proceed, dev2.try_step());
            assert_eq!(StepDirective::Proceed, dev0.try_step());
        }
        for _ in 0..30 {
            assert_eq!(StepDirective::Proceed, dev0.try_step());
            assert_eq!(StepDirective::Proceed, dev2.try_step());
            assert_eq!(StepDirective::Proceed, dev1.try_step());
        }
    }
}
