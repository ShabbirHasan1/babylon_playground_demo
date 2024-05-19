use crate::{
    id::{HandleId, SmartId},
    instruction::{InstructionError, InstructionExecutionResult, ManagedInstruction},
    types::Priority,
    util::new_id_u64,
};
use chrono::{DateTime, Utc};
use thiserror::Error;

#[derive(Error, Clone, Debug, Eq, PartialEq)]
pub enum ProcessError {
    #[error("Pending instruction not found for process {0}")]
    PendingInstructionNotFound(u64),

    #[error(transparent)]
    InstructionError(#[from] InstructionError),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum ProcessState {
    #[default]
    New,
    Ready,
    Running,
    Failed,
    Completed,
}

#[derive(Debug, Clone, Eq, PartialEq, Copy, Default)]
pub struct ProcessMetadata {
    pub parent_id:      Option<u64>,
    pub trade_id:       Option<HandleId>,
    pub local_order_id: Option<SmartId>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Process<T>
where
    T: Clone + 'static + Default + Send,
{
    pub id:                     u64,
    pub priority:               Priority, // The priority of the process is the highest priority of all instructions
    pub state:                  ProcessState,
    pub instructions:           Vec<ManagedInstruction>,
    pub pending_instruction_id: usize,
    pub instructions_completed: usize,
    pub context:                T,
    pub created_at:             DateTime<Utc>,
    pub started_at:             Option<DateTime<Utc>>,
    pub finished_at:            Option<DateTime<Utc>>,
}

impl<T> Process<T>
where
    T: Clone + 'static + Default + Send,
{
    pub fn new(metadata: ProcessMetadata, initial_context: T, mut instructions: Vec<ManagedInstruction>) -> Self {
        let new_process_id = new_id_u64();

        // The process priority is equal to the highest priority of all instructions
        let priority = instructions
            .iter()
            .max_by(|a, b| a.priority.cmp(&b.priority))
            .map(|i| i.priority)
            .unwrap_or(Priority::default());

        Self {
            id: new_id_u64(),
            priority,
            state: ProcessState::New,
            instructions,
            pending_instruction_id: 0,
            instructions_completed: 0,
            context: initial_context,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
        }
    }

    pub fn pending_instruction(&self) -> Option<&ManagedInstruction> {
        self.instructions.get(self.pending_instruction_id)
    }

    pub fn pending_instruction_mut(&mut self) -> Option<&mut ManagedInstruction> {
        self.instructions.get_mut(self.pending_instruction_id)
    }

    pub fn confirm_instruction_execution(&mut self, result: InstructionExecutionResult) -> Result<(), ProcessError> {
        // FIXME: finish this
        /*
        let instruction = self.pending_instruction_mut().ok_or(ProcessError::PendingInstructionNotFound(self.id))?;

        instruction.confirm_execution(result)?;

        self.instructions_completed += 1;
        self.pending_instruction_id += 1;
         */

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_new() {
        let process = Process::new(
            ProcessMetadata {
                parent_id:      None,
                trade_id:       None,
                local_order_id: None,
            },
            (),
            vec![],
        );

        println!("{:#?}", process);

        assert_eq!(process.priority, Priority::default());
        assert_eq!(process.state, ProcessState::New);
        assert_eq!(process.instructions, vec![]);
        assert_eq!(process.pending_instruction_id, 0);
        assert_eq!(process.instructions_completed, 0);
        assert_eq!(process.context, ());
    }
}
