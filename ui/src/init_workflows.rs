use std::future::Future;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use futures::channel::mpsc;
use iced::Task;
use updater_core::{Config, PackageManagerType};

#[derive(Debug, Clone)]
pub struct InitProgress {
    pub completed: usize,
    pub total: usize,
    pub manager: PackageManagerType,
    pub command_message: String,
}

#[derive(Debug, Clone)]
enum InitEvent<Item> {
    Started {
        total: usize,
        manager: PackageManagerType,
        command_message: String,
    },
    Item {
        manager: PackageManagerType,
        result: Result<Item, String>,
    },
    Completed {
        total: usize,
        manager: PackageManagerType,
        command_message: String,
    },
    Finished,
}

pub fn run_manager_init_task<
    Message,
    Item,
    Work,
    WorkFuture,
    ItemMessage,
    ProgressMessage,
    DoneMessage,
>(
    config: Config,
    managers: Vec<PackageManagerType>,
    start_label: impl Fn(PackageManagerType) -> String + Copy + Send + 'static,
    complete_label: impl Fn(PackageManagerType, &Result<Item, String>) -> String + Copy + Send + 'static,
    work: Work,
    item_message: ItemMessage,
    progress_message: ProgressMessage,
    done_message: DoneMessage,
) -> Task<Message>
where
    Message: Send + 'static,
    Item: Send + 'static,
    Work: Fn(PackageManagerType, Config) -> WorkFuture + Copy + Send + 'static,
    WorkFuture: Future<Output = Result<Item, String>> + Send + 'static,
    ItemMessage: Fn(PackageManagerType, Result<Item, String>) -> Message + Copy + Send + 'static,
    ProgressMessage: Fn(InitProgress) -> Message + Copy + Send + 'static,
    DoneMessage: Fn() -> Message + Copy + Send + 'static,
{
    let total = managers.len();
    if total == 0 {
        return Task::done(done_message());
    }

    let (sender, receiver) = mpsc::unbounded::<InitEvent<Item>>();
    let finished_count = Arc::new(AtomicUsize::new(0));
    let completed_count = Arc::new(AtomicUsize::new(0));
    let completed_count_for_progress = Arc::clone(&completed_count);

    let progress_task = Task::run(receiver, move |event| match event {
        InitEvent::Started {
            total,
            manager,
            command_message,
        } => progress_message(InitProgress {
            completed: completed_count_for_progress.load(Ordering::Relaxed),
            total,
            manager,
            command_message,
        }),
        InitEvent::Item { manager, result } => item_message(manager, result),
        InitEvent::Completed {
            total,
            manager,
            command_message,
        } => {
            let completed = completed_count_for_progress.fetch_add(1, Ordering::Relaxed) + 1;
            progress_message(InitProgress {
                completed,
                total,
                manager,
                command_message,
            })
        }
        InitEvent::Finished => done_message(),
    });

    let mut tasks = Vec::with_capacity(total + 1);
    for manager in managers {
        let sender_for_task = sender.clone();
        let config = config.clone();
        let finished_count_for_task = Arc::clone(&finished_count);

        let task = Task::future(async move {
            let _ = sender_for_task.unbounded_send(InitEvent::Started {
                total,
                manager,
                command_message: start_label(manager),
            });

            let result = work(manager, config).await;
            let completed_message = complete_label(manager, &result);

            let _ = sender_for_task.unbounded_send(InitEvent::Item { manager, result });
            let _ = sender_for_task.unbounded_send(InitEvent::Completed {
                total,
                manager,
                command_message: completed_message,
            });

            let finished = finished_count_for_task.fetch_add(1, Ordering::AcqRel) + 1;
            if finished == total {
                let _ = sender_for_task.unbounded_send(InitEvent::Finished);
            }
        })
        .discard();

        tasks.push(task);
    }

    tasks.push(progress_task);
    Task::batch(tasks)
}
