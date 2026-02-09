# Task Scheduler - Implementation Guide & Examples

## Quick Start

### 1. Creating a Task in GUI
1. Open **Scheduler** from the sidebar
2. Click **"New Task"**
3. Fill in:
   - **Repository**: Your backup repository name
   - **Archive**: Name for the archive being created
   - **Schedule Type**: Daily, Weekly, or Manual
   - **Time**: Hour (0-23) and Minute (0-59)
4. Click **"Save"**

### 2. Testing Scheduled Execution
```bash
# Monitor daemon logs
journalctl -u borgd -f

# In another terminal, manually trigger a job
borgd run-job:your-task-name

# Check daemon status
borgd status
```

---

## How Scheduling Works

### Daily Schedule Example
```
Schedule Type: Daily
Hour: 3 (3 AM)
Minute: 0
```
**Execution**: 03:00 every single day

### Weekly Schedule Example
```
Schedule Type: Weekly
Weekday: Monday (0)
Hour: 14 (2 PM)
Minute: 30
```
**Execution**: Every Monday at 14:30

### Manual Schedule Example
```
Schedule Type: Manual
```
**Execution**: Only when explicitly triggered via:
- `borgd run-job:task-name` command
- UI manual trigger button (if implemented)

---

## Daemon Scheduler Behavior

### Job Execution Timeline
```
T-2m: Daemon checks schedule, sees job in 2 minutes
T-1m: Daemon still waiting (checks every 30 seconds)
T-30s: Daemon detects job within 60 seconds, sleeps
T-0s: Job executes at exact scheduled time
T+0-5s: Job execution completes
T+5s: Daemon reschedules job for next occurrence
T+30s: Next scheduler loop runs
```

### Missed Job Handling
```
If daemon is offline from 3:00 to 6:00 (when 3 AM backup should run):

1. Daemon starts at 6:15
2. Scheduler immediately detects overdue job
3. If "Run on boot if missed" is enabled: Execute immediately
4. If "Run on boot if missed" is disabled: Skip this occurrence
5. Reschedule for next daily/weekly occurrence
```

---

## Configuration Examples

### GUI Task File Structure
File: `~/.config/borg-gui/scheduled_tasks.json`

```json
[
  {
    "repo_name": "home-backup",
    "archive_name": "documents",
    "schedule": {
      "schedule_type": "Daily",
      "weekday": 0,
      "hour": 3,
      "minute": 0,
      "run_on_boot_if_missed": true
    }
  },
  {
    "repo_name": "media-backup",
    "archive_name": "photos",
    "schedule": {
      "schedule_type": "Weekly",
      "weekday": 5,
      "hour": 22,
      "minute": 30,
      "run_on_boot_if_missed": false
    }
  }
]
```

### Daemon Cron Configuration
File: `/etc/borg-rust/borgd.yaml`

```yaml
jobs:
  - name: "daily-home-backup"
    enabled: true
    priority: 10
    schedule: "0 3 * * *"  # Daily at 3 AM
    repository: "/mnt/backup/home"
    paths:
      - /home/user/documents
      - /home/user/pictures
    archive_name: "home-{now}"
    
  - name: "weekly-full-backup"
    enabled: true
    priority: 5
    schedule: "0 22 * * 5"  # Friday at 10 PM
    repository: "/mnt/backup/full"
    paths:
      - /home
      - /etc
    archive_name: "full-{now}"
```

---

## Cron Expression Syntax

### Common Patterns
```
Field       Allowed Values
─────       ──────────────
Minute      0–59
Hour        0–23
Day         1–31
Month       1–12 (JAN–DEC)
Weekday     0–7 (0 and 7 are SUN)

Examples:
0 3 * * *          → 3:00 AM every day
0 14 * * 1         → 2:00 PM every Monday
0 */6 * * *        → Every 6 hours (00:00, 06:00, 12:00, 18:00)
30 2 * * 1-5       → 2:30 AM Monday–Friday
0 0 1 * *          → Midnight on the 1st of each month
```

---

## Troubleshooting

### Problem: Task Created but Never Executes
**Solution**:
1. Verify daemon is running: `systemctl status borgd`
2. Check logs: `journalctl -u borgd -n 20`
3. Ensure task time hasn't already passed
4. Verify repository and archive names are valid

### Problem: Task Executes Too Late
**Solution**:
- Scheduler checks every 30 seconds, so max delay is ~30-60 seconds
- This is normal and expected behavior
- If tighter timing is needed, use cron jobs instead

### Problem: Task Executes Multiple Times
**Solution**:
- Check if "run_on_boot_if_missed" is enabled when it shouldn't be
- Verify only one instance of daemon is running
- Check daemon logs for execution records

### Problem: Changes to Task Don't Take Effect
**Solution**:
1. Save task in GUI
2. Verify file: `cat ~/.config/borg-gui/scheduled_tasks.json`
3. Restart daemon: `systemctl restart borgd`
4. Check new schedule: `borgd list-jobs`

---

## Advanced Usage

### Scheduling with Dependencies
Create multiple tasks with different times to establish sequence:
```
Task 1: "backup-system" at 02:00 (system files)
Task 2: "backup-data"   at 03:00 (user data - depends on Task 1)
Task 3: "backup-media"  at 04:00 (media files - depends on Task 2)
```

### Priority-Based Scheduling
Use priority field in daemon config (lower number = higher priority):
```yaml
jobs:
  - name: "critical-backup"
    priority: 1        # Executes first if multiple jobs are ready
    schedule: "0 2 * * *"
    
  - name: "standard-backup"
    priority: 10       # Executes second
    schedule: "0 2 * * *"
```

### Monitoring with Systemd
```bash
# View real-time logs
journalctl -u borgd -f

# View last 100 lines
journalctl -u borgd -n 100

# View since last boot
journalctl -u borgd -b

# View only errors
journalctl -u borgd -p err
```

---

## Performance Tuning

### Reduce Polling Interval
If you need more precise timing (currently 30 seconds):
Edit `borg-daemon/src/main.rs` line 328:
```rust
// Change from:
tokio::time::sleep(std::time::Duration::from_secs(30)) => {

// To (for 10 seconds):
tokio::time::sleep(std::time::Duration::from_secs(10)) => {
```
Tradeoff: More CPU usage, better timing precision

### Batch Multiple Tasks
Instead of multiple separate tasks, create one task that runs a wrapper script:
```bash
#!/bin/bash
# /etc/borg-rust/backup-all.sh
borgd run-job:backup-data &
borgd run-job:backup-media &
wait
```

---

## API Reference

### Scheduler Methods (Daemon)
```rust
// Get next scheduled job
pub fn next_job(&self) -> Option<(String, DateTime<Utc>)>

// Mark job as completed and reschedule
pub fn job_completed(&mut self, job_name: &str)

// Mark job as failed (for missed run tracking)
pub fn job_failed(&mut self, job_name: &str)

// Schedule job to run immediately
pub fn schedule_now(&mut self, job_name: &str) -> bool

// Get expected runs between timestamps
pub fn expected_runs(&self, job_name: &str, after: DateTime<Utc>, before: DateTime<Utc>) -> Vec<DateTime<Utc>>
```

---

## Integration with External Tools

### Slack Notifications
```bash
#!/bin/bash
# /etc/borg-rust/notify-slack.sh
curl -X POST https://hooks.slack.com/services/YOUR/WEBHOOK/URL \
  -d '{"text":"Backup completed: '"$1"'"}'
```

### Email Notifications
```bash
#!/bin/bash
# /etc/borg-rust/notify-email.sh
echo "Backup $1 completed" | mail -s "Backup Status" admin@example.com
```

---

## Best Practices

1. **Set Multiple Backups**: Don't rely on single daily backup
2. **Off-Peak Scheduling**: Schedule during low-activity hours
3. **Monitor Regularly**: Check logs weekly for failures
4. **Test Restore**: Verify backups can be restored
5. **Graduated Retention**: Use different schedules for daily/weekly/monthly
6. **Document Your Schedule**: Keep notes on why each backup is scheduled

---

## Related Documentation

- **Archive Files Dialog**: Allows browsing archive contents after creation
- **Repository Management**: GUI for managing backup repositories
- **Compression Settings**: Configure compression per archive
- **Recovery Procedures**: How to restore from backups


