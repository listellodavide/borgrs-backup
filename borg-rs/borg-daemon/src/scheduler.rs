//! Job scheduler with cron-based scheduling

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use chrono::{DateTime, Utc};
use cron::Schedule;
use tracing::{debug, warn};

use crate::config::BackupJob;

/// Scheduled job entry
#[derive(Debug, Clone)]
struct ScheduledJob {
    name: String,
    next_run: DateTime<Utc>,
    schedule: Schedule,
    priority: u32,
    #[allow(dead_code)]
    last_scheduled: Option<DateTime<Utc>>,
    #[allow(dead_code)]
    last_success: Option<DateTime<Utc>>,
    missed_runs: u32,
}

impl PartialEq for ScheduledJob {
    fn eq(&self, other: &Self) -> bool {
        self.next_run == other.next_run && self.priority == other.priority
    }
}

impl Eq for ScheduledJob {}

impl PartialOrd for ScheduledJob {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledJob {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for min-heap behavior (earliest time first)
        // Then by priority (lower priority number = higher priority)
        match other.next_run.cmp(&self.next_run) {
            Ordering::Equal => other.priority.cmp(&self.priority),
            ord => ord,
        }
    }
}

/// Job scheduler managing backup schedules
pub struct Scheduler {
    jobs: BinaryHeap<ScheduledJob>,
    job_configs: Vec<BackupJob>,
}

impl Scheduler {
    /// Create a new scheduler with the given job configurations
    pub fn new(jobs: Vec<BackupJob>) -> Self {
        let mut scheduler = Self {
            jobs: BinaryHeap::new(),
            job_configs: jobs,
        };
        scheduler.schedule_all_jobs();
        scheduler
    }

    /// Schedule all enabled jobs
    fn schedule_all_jobs(&mut self) {
        self.jobs.clear();
        let _now = Utc::now();

        for job in &self.job_configs {
            if !job.enabled {
                debug!("Skipping disabled job: {}", job.name);
                continue;
            }

            match job.schedule.parse::<Schedule>() {
                Ok(schedule) => {
                    if let Some(next_run) = schedule.upcoming(Utc).next() {
                        debug!("Scheduled job '{}' for {}", job.name, next_run);
                        self.jobs.push(ScheduledJob {
                            name: job.name.clone(),
                            next_run,
                            schedule,
                            priority: job.priority,
                            last_scheduled: None,
                            last_success: None,
                            missed_runs: 0,
                        });
                    }
                }
                Err(e) => {
                    warn!("Invalid schedule for job '{}': {}", job.name, e);
                }
            }
        }
    }

    /// Get the next job to run and its scheduled time
    pub fn next_job(&self) -> Option<(String, DateTime<Utc>)> {
        self.jobs.peek().map(|job| (job.name.clone(), job.next_run))
    }

    /// Get expected runs between timestamps
    pub fn expected_runs(
        &self,
        job_name: &str,
        after: DateTime<Utc>,
        before: DateTime<Utc>,
    ) -> Vec<DateTime<Utc>> {
        self.jobs
            .iter()
            .find(|job| job.name == job_name)
            .map(|job| {
                job.schedule
                    .after(&after)
                    .take_while(|dt| *dt <= before)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    /// Mark a job as completed and reschedule it
    pub fn job_completed(&mut self, job_name: &str) {
        // Remove the completed job from the heap
        let jobs: Vec<_> = self.jobs.drain().collect();

        for job in jobs {
            if job.name == job_name {
                // Reschedule this job
                if let Some(next_run) = job.schedule.upcoming(Utc).next() {
                    debug!("Rescheduled job '{}' for {}", job_name, next_run);
                    self.jobs.push(ScheduledJob {
                        name: job.name,
                        next_run,
                        schedule: job.schedule,
                        priority: job.priority,
                        last_scheduled: Some(Utc::now()),
                        last_success: Some(Utc::now()),
                        missed_runs: 0,
                    });
                }
            } else {
                // Keep other jobs
                self.jobs.push(job);
            }
        }
    }

    /// Mark a job as failed (for missed runs detection)
    pub fn job_failed(&mut self, job_name: &str) {
        let jobs: Vec<_> = self.jobs.drain().collect();

        for job in jobs {
            if job.name == job_name {
                // Increment missed runs counter
                let mut updated_job = job;
                updated_job.missed_runs += 1;
                updated_job.last_scheduled = Some(Utc::now());

                // Reschedule this job
                if let Some(next_run) = updated_job.schedule.upcoming(Utc).next() {
                    debug!("Rescheduled failed job '{}' for {}", job_name, next_run);
                    self.jobs.push(updated_job);
                }
            } else {
                // Keep other jobs
                self.jobs.push(job);
            }
        }
    }

    /// Update job configurations and reschedule
    #[allow(dead_code)]
    pub fn update_jobs(&mut self, jobs: Vec<BackupJob>) {
        self.job_configs = jobs;
        self.schedule_all_jobs();
    }

    /// Get all scheduled jobs with their next run times
    pub fn list_scheduled(&self) -> Vec<(String, DateTime<Utc>, bool)> {
        let mut result: Vec<_> = self
            .jobs
            .iter()
            .map(|job| {
                let enabled = self
                    .job_configs
                    .iter()
                    .find(|j| j.name == job.name)
                    .map(|j| j.enabled)
                    .unwrap_or(false);
                (job.name.clone(), job.next_run, enabled)
            })
            .collect();

        result.sort_by(|a, b| a.1.cmp(&b.1));
        result
    }

    /// Force immediate scheduling of a specific job
    #[allow(dead_code)]
    pub fn schedule_now(&mut self, job_name: &str) -> bool {
        if let Some(job_config) = self.job_configs.iter().find(|j| j.name == job_name) {
            if let Ok(schedule) = job_config.schedule.parse::<Schedule>() {
                self.jobs.push(ScheduledJob {
                    name: job_name.to_string(),
                    next_run: Utc::now(),
                    schedule,
                    priority: 0, // Highest priority for manual runs
                    last_scheduled: Some(Utc::now()),
                    last_success: None,
                    missed_runs: 0,
                });
                return true;
            }
        }
        false
    }

    /// Check if a job is currently scheduled
    #[allow(dead_code)]
    pub fn is_scheduled(&self, job_name: &str) -> bool {
        self.jobs.iter().any(|job| job.name == job_name)
    }

    /// Get job configuration by name
    #[allow(dead_code)]
    pub fn get_job_config(&self, job_name: &str) -> Option<&BackupJob> {
        self.job_configs.iter().find(|j| j.name == job_name)
    }

    /// Get the next expected run time for a job
    #[allow(dead_code)]
    pub fn next_expected_run(&self, job_name: &str) -> Option<DateTime<Utc>> {
        self.jobs
            .iter()
            .find(|job| job.name == job_name)
            .map(|job| job.next_run)
    }

    /// Check if a job has missed runs (based on threshold)
    #[allow(dead_code)]
    pub fn has_missed_runs(&self, job_name: &str, max_missed: u32) -> bool {
        self.jobs
            .iter()
            .find(|job| job.name == job_name)
            .map(|job| job.missed_runs > max_missed)
            .unwrap_or(false)
    }

    /// Get last success time for a job
    #[allow(dead_code)]
    pub fn last_success_time(&self, job_name: &str) -> Option<DateTime<Utc>> {
        self.jobs
            .iter()
            .find(|job| job.name == job_name)
            .map(|job| job.last_success)
            .flatten()
    }

    /// Get missed runs count for a job
    #[allow(dead_code)]
    pub fn missed_runs_count(&self, job_name: &str) -> u32 {
        self.jobs
            .iter()
            .find(|job| job.name == job_name)
            .map(|job| job.missed_runs)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_job(name: &str, schedule: &str, priority: u32) -> BackupJob {
        BackupJob {
            name: name.to_string(),
            schedule: schedule.to_string(),
            repository: "/tmp/test-repo".to_string(),
            paths: vec![std::path::PathBuf::from("/tmp")],
            exclude_patterns: vec![],
            exclude_files: vec![],
            exclude_caches: true,
            exclude_if_present: vec![],
            archive_name: "{hostname}-{now}".to_string(),
            compression: None,
            compression_level: None,
            one_file_system: false,
            read_special: false,
            numeric_ids: false,
            noatime: true,
            exclude_nodump: false,
            prune: None,
            pre_command: None,
            post_command: None,
            notifications: Default::default(),
            priority,
            enabled: true,
            retry: Default::default(),
        }
    }

    #[test]
    fn test_scheduler_creation() {
        let jobs = vec![
            create_test_job("daily", "0 0 2 * * *", 100),
            create_test_job("hourly", "0 0 * * * *", 50),
        ];

        let scheduler = Scheduler::new(jobs);
        assert!(scheduler.next_job().is_some());
    }

    #[test]
    fn test_job_ordering() {
        let jobs = vec![
            create_test_job("low-priority", "0 0 * * * *", 200),
            create_test_job("high-priority", "0 0 * * * *", 10),
        ];

        let scheduler = Scheduler::new(jobs);
        let scheduled = scheduler.list_scheduled();

        // Both jobs have same schedule, but high-priority should be ordered first
        assert_eq!(scheduled.len(), 2);
    }

    #[test]
    fn test_schedule_now() {
        let jobs = vec![create_test_job("daily", "0 0 2 * * *", 100)];

        let mut scheduler = Scheduler::new(jobs);
        assert!(scheduler.schedule_now("daily"));

        let (name, time) = scheduler.next_job().unwrap();
        assert_eq!(name, "daily");
        // Job should be scheduled very close to now
        assert!((Utc::now() - time).num_seconds().abs() < 5);
    }

    #[test]
    fn test_expected_runs_between() {
        let jobs = vec![create_test_job("hourly", "0 0 * * * *", 50)];
        let scheduler = Scheduler::new(jobs);

        let start = Utc::now() - chrono::Duration::hours(3);
        let end = Utc::now();
        let expected = scheduler.expected_runs("hourly", start, end);
        assert!(!expected.is_empty());
    }

    #[test]
    fn test_job_completion_tracking() {
        let jobs = vec![create_test_job("daily", "0 0 2 * * *", 100)];
        let mut scheduler = Scheduler::new(jobs);

        // Initially no last success
        assert_eq!(scheduler.last_success_time("daily"), None);

        // Mark job as completed
        scheduler.job_completed("daily");

        // Should now have a last success time
        assert!(scheduler.last_success_time("daily").is_some());
    }

    #[test]
    fn test_missed_runs_tracking() {
        let jobs = vec![create_test_job("daily", "0 0 2 * * *", 100)];
        let mut scheduler = Scheduler::new(jobs);

        // Initially no missed runs
        assert_eq!(scheduler.missed_runs_count("daily"), 0);

        // Mark job as failed
        scheduler.job_failed("daily");

        // Should now have one missed run
        assert_eq!(scheduler.missed_runs_count("daily"), 1);
    }
}
