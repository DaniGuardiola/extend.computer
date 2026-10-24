"""Select only an app's native notification controls in an open Android drawer.

Collapsed summaries can belong to SystemUI while expanded RemoteViews belong to
the posting app. App-page buttons must never count as notification actions.
"""
import re


def _label(node, value):
    return value in (node.get("text"), node.get("description"))


def _rect(node):
    try:
        left, top, right, bottom = map(int, node.get("bounds", "").split())
    except (TypeError, ValueError):
        return None
    if left < 0 or top < 0 or right <= left or bottom <= top:
        return None
    return left, top, right, bottom


def notification_step(own, system, title, action):
    """Return the exact app action or nearest own expansion control, if visible."""
    title_ids = {"android:id/title", "com.android.systemui:id/notification_title"}
    own_titles = [node for node in own if _label(node, title) and node.get("resource") in title_ids]
    summary_titles = [node for node in system if _label(node, title) and node.get("resource") in title_ids]
    if len(own_titles) > 1 or len(summary_titles) > 1:
        raise RuntimeError("Notification title is ambiguous")
    actions = [node for node in own if _label(node, action) and node.get("clickable")
               and re.fullmatch(r"android:id/action\d+", node.get("resource") or "")]
    if len(actions) > 1:
        raise RuntimeError("Notification action is ambiguous")
    if own_titles and actions:
        return {"kind": "action", "node": actions[0]}
    titles = own_titles or summary_titles
    if not titles:
        return None
    rect = _rect(titles[0])
    if rect is None:
        return None
    title_y = (rect[1] + rect[3]) / 2
    expansions = []
    for node in own:
        bounds = _rect(node)
        if (node.get("description") == "Expand" and node.get("clickable")
                and node.get("resource") == "android:id/expand_button" and bounds
                and bounds[0] >= rect[0]):
            # Prefer this title's row; a collapsed group uses its nearby header.
            distance = 0 if bounds[1] <= title_y <= bounds[3] else min(abs(title_y - bounds[1]), abs(title_y - bounds[3]))
            expansions.append((distance, node))
    if not expansions:
        return None
    expansions.sort(key=lambda item: item[0])
    if len(expansions) > 1 and expansions[0][0] == expansions[1][0]:
        raise RuntimeError("Notification expansion is ambiguous")
    return {"kind": "expand", "node": expansions[0][1]}
