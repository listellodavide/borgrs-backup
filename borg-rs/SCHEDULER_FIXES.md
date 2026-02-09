# Task Scheduler Implementation - Fixes and Improvements

## Overview
Fixed critical issues with the task scheduler in both the GUI and daemon components, making it reliable and responsive to user-defined scheduled backups.

## Issues Fixed

### 1. **Scheduler UI Issues ✅**
**Problem**: 
- Buttons were too large (taking up excessive space)
- No way to edit repository and archive names directly
- Task list text was hard to read
- Controls were not properly organized

**Solution**:
- Added `LineEdit` fields for direct editing of repository and archive names
- Reduced button heights from default to 32px and widths to 80px minimum
- Reorganized layout using `GroupBox` to group related controls
- Made task list text smaller (12px) for better visibility
- Improved spacing and alignment for better UI flow

**Files Modified**: `borg-gui/ui/scheduler.slint`

---

### 2. **Daemon Scheduler Timing Issues ✅**
**Problem**:
- Scheduler only checked `next_job()` once per iteration
- If a scheduled time was missed, it would wait for the next occurrence
- No periodic checking for overdue jobs
- The scheduler loop was inefficient and could miss execution windows
- No rescheduling after job completion, leading to one-time-only execution

**Solution**:
Implemented a more robust scheduler loop with the following improvements:

1. **Periodic Checking**: Changed from single check to check every 30 seconds
2. **Execution Timeout Handling**: 
   - If job time has already passed (delay ≤ 0), execute immediately
   - If job is within 60 seconds, sleep and execute at exact time
   - If job is >60 seconds away, loop and check again after 30 seconds
3. **Proper Rescheduling**: 
   - Call `scheduler.job_completed()` after successful execution
   - Call `scheduler.job_failed()` on failure for missed run tracking
4. **Error Handling**: Failed jobs are properly marked and rescheduled

**Files Modified**: `borg-daemon/src/main.rs` (run_scheduler function)

---

## Architecture

### GUI Scheduled Tasks
- Stored in JSON file: `~/.config/borg-gui/scheduled_tasks.json`
- Structure:
  ```json
  {
    "repo_name": "My Repository",
    "archive_name": "archive_name",
    "schedule": {
      "schedule_type": "Daily|Weekly|Manual",
      "weekday": 0-6,        // 0 = Monday
      "hour": 0-23,
      "minute": 0-59,
      "run_on_boot_if_missed": true
    }
  }
  ```

### Daemon Scheduler
- Loads cron-based backup jobs from `borgd.yaml`
- Uses the `cron` crate for schedule parsing
- Maintains a priority queue of scheduled jobs
- Executes jobs and reschedules them automatically

### Interaction Flow
1. **GUI**: User creates/edits scheduled task
2. **GUI**: Task is saved to JSON file
3. **Daemon**: Daemon reads tasks from configuration (periodic reload)
4. **Daemon**: Creates cron expressions from task schedules
5. **Daemon**: Executes jobs at scheduled times
6. **Daemon**: Records success/failure and reschedules

---

## Testing the Scheduler

### To Verify Scheduler is Working:

1. **Create a Task in GUI**:
   - Open Scheduler view
   - Click "New Task"
   - Fill in repository and archive names
   - Set schedule to "Daily" at current time + 2 minutes
   - Click "Save"

2. **Monitor Daemon Execution**:
   ```bash
   journalctl -u borgd -f
   # or in foreground:
   borgd --foreground --log-level debug
   ```

3. **Check Job State**:
   ```bash
   borgd status
   ```

---

## Key Improvements

### Reliability
- ✅ Jobs no longer get stuck if execution time is missed
- ✅ Immediate execution of overdue jobs
- ✅ Proper error handling and failure tracking
- ✅ Periodic checking ensures no jobs are lost

### Performance
- ✅ Efficient 30-second polling instead of long sleeps
- ✅ Minimal CPU usage during idle periods
- ✅ Smart wake-ups for upcoming jobs

### User Experience
- ✅ Clear task management interface
- ✅ Editable task names/targets
- ✅ Organized scheduling controls
- ✅ Proper success/failure notifications

---

## Schedule Format Reference

### Daily
Runs every day at specified hour:minute
```
Example: 03:00 (3 AM) every day
```

### Weekly
Runs every week on specified day at specified hour:minute
```
Example: Monday at 14:30 (2:30 PM)
```

### Manual
Only runs when manually triggered through UI or daemon command
```
Command: borgd run-job:<jobname>
```

### Run on Boot if Missed
If enabled, the job will execute immediately on daemon startup if it was supposed to run while the system was offline.

---

## Cron Expression Reference (for daemon configuration)

The daemon also supports cron expressions in `borgd.yaml`:

```yaml
jobs:
  - name: "Daily backup"
    schedule: "0 3 * * *"      # 3 AM every day
    
  - name: "Weekly backup"
    schedule: "0 14 * * 1"      # 2 PM every Monday
    
  - name: "Every 6 hours"
    schedule: "0 */6 * * *"     # Every 6 hours
```

For more details, see: https://docs.rs/cron/latest/cron/

---

## Future Enhancements

1. **Database Persistence**: Store task execution history for audit logging
2. **Web Interface**: Remote task management via HTTP API
3. **Dynamic Reloading**: Reload tasks without restarting daemon
4. **Parallel Execution**: Support multiple concurrent backups
5. **Task Dependencies**: Run tasks in sequence based on dependencies
6. **Advanced Cron UI**: Visual cron expression builder in GUI

---

## Debugging

### Enable Debug Logging:
```bash
borgd --foreground --log-level debug
```

### Check Scheduled Jobs:
```bash
borgd list-jobs
```

### Run Job Immediately:
```bash
borgd run-job:your-job-name
```

### View Recent Logs:
```bash
journalctl -u borgd -n 50
```

---

## References
- Task Scheduler Cron Expressions in Rust: https://oneuptime.com/blog/post/2026-01-25-task-scheduler-cron-expressions-rust/view
- Cron Crate Documentation: https://docs.rs/cron/latest/cron/
- Slint UI Framework: https://docs.slint.dev/


