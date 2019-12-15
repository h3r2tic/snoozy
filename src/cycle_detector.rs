use std::sync::{
    atomic,
    mpsc::{channel, Receiver, Sender},
    Arc, Mutex,
};

type CycleDetectorAddEdgeCmd = (usize, usize);

pub struct CycleDetectorBackend {
    cmd_channel: Receiver<CycleDetectorAddEdgeCmd>,
    cycle_found: Arc<atomic::AtomicBool>,
    graph: petgraph::graphmap::DiGraphMap<usize, ()>,
}

impl CycleDetectorBackend {
    pub fn run(mut self) {
        while let Ok((v0, v1)) = self.cmd_channel.recv() {
            self.graph.add_node(v0);
            self.graph.add_node(v1);
            self.graph.add_edge(v0, v1, ());

            if petgraph::algo::is_cyclic_directed(&self.graph) {
                // TODO: report the exact cycle
                self.cycle_found.store(true, atomic::Ordering::Relaxed);
                println!("Cycle found! Bailing! Make sure to use .prev() on refs");
                std::process::exit(1);
                //break;
            }
        }
    }
}

pub struct CycleDetector {
    cycle_found: Arc<atomic::AtomicBool>,

    // Required due to https://github.com/rust-lang/rust/issues/57017
    // The compiler too eagerly captures references to this type,
    // and wants &Sender to be Send, thus Sender to be Sync.
    // Only ever accessed via get_mut().
    cmd_channel: Mutex<Sender<CycleDetectorAddEdgeCmd>>,
}

impl Clone for CycleDetector {
    fn clone(&self) -> Self {
        Self {
            cycle_found: self.cycle_found.clone(),
            cmd_channel: Mutex::new(self.cmd_channel.lock().unwrap().clone()),
        }
    }
}

pub fn create_cycle_detector() -> (CycleDetector, CycleDetectorBackend) {
    let cycle_found = Arc::new(atomic::AtomicBool::new(false));
    let (tx, rx) = channel();
    (
        CycleDetector {
            cycle_found: cycle_found.clone(),
            cmd_channel: Mutex::new(tx),
        },
        CycleDetectorBackend {
            cycle_found: cycle_found.clone(),
            cmd_channel: rx,
            graph: Default::default(),
        },
    )
}

impl CycleDetector {
    /*fn get_cycle_tracker(&self) -> Arc<atomic::AtomicBool> {
        self.cycle_found.clone()
    }*/

    pub fn add_edge(&mut self, v0: usize, v1: usize) {
        let _ = self.cmd_channel.get_mut().unwrap().send((v0, v1));
    }
}
