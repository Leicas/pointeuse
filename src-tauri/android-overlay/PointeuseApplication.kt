package __APP_PACKAGE__

import android.app.Application
import androidx.work.Configuration

/**
 * Application class whose only job is to make WorkManager's on-demand
 * initialization work from every process entry point.
 *
 * The overlay manifest removes `androidx.work.WorkManagerInitializer` so the
 * app can install [AppWorkerFactory]. Doing that initialization in
 * MainActivity.onCreate is not enough: when the schedule-task plugin's
 * WorkManager job fires while the app is closed, Android starts this process
 * with only `androidx.work.impl.background.systemjob.SystemJobService` — no
 * activity ever runs — and WorkManager.getInstance() throws
 * "WorkManager is not initialized properly … your Application does not
 * implement Configuration.Provider", killing the process. The user sees
 * "Pointeuse crashed unexpectedly" without having opened the app, and the
 * job is retried a few seconds later with the same result.
 *
 * Implementing [Configuration.Provider] here lets WorkManager initialize
 * itself lazily, with our factory, from the service, a receiver or the
 * activity alike.
 */
class PointeuseApplication : Application(), Configuration.Provider {
    override val workManagerConfiguration: Configuration
        get() = Configuration.Builder()
            .setWorkerFactory(AppWorkerFactory())
            .build()
}