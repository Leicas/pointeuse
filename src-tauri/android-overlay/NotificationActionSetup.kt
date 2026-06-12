package __APP_PACKAGE__

import android.content.Context
import android.content.SharedPreferences
import app.tauri.Logger

/**
 * Registers notification action types directly in SharedPreferences,
 * working around a bug in tauri-plugin-notification's writeActionGroup
 * where the write key uses action type ID instead of numeric index.
 */
object NotificationActionSetup {

    private const val ACTION_TYPES_ID = "ACTION_TYPE_STORE"

    fun registerReminderActions(context: Context) {
        val actions = listOf(
            Triple("continue", "Continue", false),
            Triple("change_task", "Change Task", false),
            Triple("stop", "Stop", false)
        )

        val prefs: SharedPreferences = context.getSharedPreferences(
            ACTION_TYPES_ID + "reminder_actions",
            Context.MODE_PRIVATE
        )

        val editor = prefs.edit()
        editor.clear()
        editor.putInt("count", actions.size)
        for ((i, action) in actions.withIndex()) {
            editor.putString("id$i", action.first)
            editor.putString("title$i", action.second)
            editor.putBoolean("input$i", action.third)
        }
        editor.apply()

        Logger.info("[NotifActions] Registered ${actions.size} reminder actions (workaround)")
    }
}
