//! Undo/redo command stack.

use wz_maplib::WzMap;

/// A reversible edit operation.
pub trait EditCommand: Send + Sync {
    fn execute(&self, map: &mut WzMap);
    fn undo(&self, map: &mut WzMap);

    /// Whether replaying this command requires rebuilding object instance
    /// buffers. Tile-height edits return `true` so undo/redo refreshes the
    /// sampled Y of any objects sitting on the affected tiles.
    fn dirties_objects(&self) -> bool {
        false
    }
}

/// Undo/redo history stack.
pub struct EditHistory {
    undo_stack: Vec<Box<dyn EditCommand>>,
    redo_stack: Vec<Box<dyn EditCommand>>,
}

impl std::fmt::Debug for EditHistory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditHistory")
            .field("undo_depth", &self.undo_stack.len())
            .field("redo_depth", &self.redo_stack.len())
            .finish()
    }
}

impl EditHistory {
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    #[expect(
        dead_code,
        reason = "symmetric counterpart to push_already_applied; tools currently apply then push"
    )]
    pub fn execute(&mut self, cmd: Box<dyn EditCommand>, map: &mut WzMap) {
        cmd.execute(map);
        self.undo_stack.push(cmd);
        self.redo_stack.clear();
    }

    /// Returns whether the replayed command dirties object instance buffers.
    pub fn undo(&mut self, map: &mut WzMap) -> bool {
        if let Some(cmd) = self.undo_stack.pop() {
            cmd.undo(map);
            let dirties_objects = cmd.dirties_objects();
            self.redo_stack.push(cmd);
            dirties_objects
        } else {
            false
        }
    }

    /// Returns whether the replayed command dirties object instance buffers.
    pub fn redo(&mut self, map: &mut WzMap) -> bool {
        if let Some(cmd) = self.redo_stack.pop() {
            cmd.execute(map);
            let dirties_objects = cmd.dirties_objects();
            self.undo_stack.push(cmd);
            dirties_objects
        } else {
            false
        }
    }

    /// Push a command that has already been applied to the map. Used when
    /// tool functions mutate the map directly and return the command.
    pub fn push_already_applied(&mut self, cmd: Box<dyn EditCommand>) {
        self.undo_stack.push(cmd);
        self.redo_stack.clear();
    }
}

/// A compound command that groups multiple edit commands into one undo/redo step.
pub struct CompoundCommand {
    commands: Vec<Box<dyn EditCommand>>,
}

impl CompoundCommand {
    pub fn new(commands: Vec<Box<dyn EditCommand>>) -> Self {
        Self { commands }
    }
}

impl EditCommand for CompoundCommand {
    fn execute(&self, map: &mut WzMap) {
        for cmd in &self.commands {
            cmd.execute(map);
        }
    }

    fn undo(&self, map: &mut WzMap) {
        for cmd in self.commands.iter().rev() {
            cmd.undo(map);
        }
    }

    fn dirties_objects(&self) -> bool {
        self.commands.iter().any(|c| c.dirties_objects())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct CounterCmd {
        counter: std::sync::Arc<AtomicU32>,
        delta: u32,
    }

    impl EditCommand for CounterCmd {
        fn execute(&self, _map: &mut WzMap) {
            self.counter.fetch_add(self.delta, Ordering::Relaxed);
        }
        fn undo(&self, _map: &mut WzMap) {
            self.counter.fetch_sub(self.delta, Ordering::Relaxed);
        }
    }

    #[test]
    fn compound_execute_and_undo() {
        let counter = std::sync::Arc::new(AtomicU32::new(0));
        let cmds: Vec<Box<dyn EditCommand>> = vec![
            Box::new(CounterCmd {
                counter: counter.clone(),
                delta: 1,
            }),
            Box::new(CounterCmd {
                counter: counter.clone(),
                delta: 10,
            }),
            Box::new(CounterCmd {
                counter: counter.clone(),
                delta: 100,
            }),
        ];
        let compound = CompoundCommand::new(cmds);

        let mut map = WzMap {
            map_data: wz_maplib::MapData::new(1, 1),
            structures: Vec::new(),
            droids: Vec::new(),
            features: Vec::new(),
            terrain_types: None,
            labels: Vec::new(),
            map_name: String::new(),
            players: 0,
            tileset: String::new(),
            custom_templates_json: None,
            author: None,
            additional_authors: Vec::new(),
            license: None,
        };
        compound.execute(&mut map);
        assert_eq!(counter.load(Ordering::Relaxed), 111);

        compound.undo(&mut map);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }
}
