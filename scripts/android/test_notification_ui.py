import unittest

from notification_ui import notification_step


TITLE = "Sharing your screen"
ACTION = "Stop sharing"


def title(resource="android:id/title", bounds="822 437 1106 480"):
    return {"text": TITLE, "resource": resource, "bounds": bounds}


def expand(bounds):
    return {"description": "Expand", "resource": "android:id/expand_button", "clickable": True, "bounds": bounds}


def stop(resource="android:id/action0"):
    return {"text": ACTION, "description": ACTION, "resource": resource, "clickable": True, "bounds": "795 624 1029 732"}


class NotificationUiTests(unittest.TestCase):
    def test_expanded_remote_views_use_posting_app_package(self):
        step = notification_step([title(), stop()], [], TITLE, ACTION)
        self.assertEqual(step["kind"], "action")
        self.assertEqual(step["node"]["resource"], "android:id/action0")

    def test_collapsed_system_summary_uses_own_group_header(self):
        header = expand("1730 284 1856 410")
        step = notification_step([header], [title("com.android.systemui:id/notification_title", "822 392 1134 438")], TITLE, ACTION)
        self.assertEqual(step, {"kind": "expand", "node": header})

    def test_child_expansion_stays_on_capture_row(self):
        capture = expand("1730 392 1856 579")
        usb = expand("1730 579 1856 766")
        step = notification_step([title(), usb, capture], [], TITLE, ACTION)
        self.assertEqual(step["node"], capture)

    def test_app_page_stop_cannot_satisfy_notification_action(self):
        self.assertIsNone(notification_step([{"text": TITLE}, stop(None)], [], TITLE, ACTION))

    def test_system_action_cannot_substitute_for_own_action(self):
        self.assertIsNone(notification_step([title()], [stop()], TITLE, ACTION))

    def test_duplicate_native_actions_fail_closed(self):
        with self.assertRaisesRegex(RuntimeError, "ambiguous"):
            notification_step([title(), stop(), stop("android:id/action1")], [], TITLE, ACTION)

    def test_invalid_expansion_geometry_is_rejected(self):
        self.assertIsNone(notification_step([title(), expand("1730 392 1730 579")], [], TITLE, ACTION))


if __name__ == "__main__":
    unittest.main()
