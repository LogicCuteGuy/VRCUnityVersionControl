using System;
using LogicCuteGuy.Vcs.Editor.Workspace;
using UnityEditor;
using UnityEngine;

namespace LogicCuteGuy.Vcs.Editor.Integration
{
    [InitializeOnLoad]
    public static class ProjectOverlay
    {
        private static GUIStyle _badgeStyle;

        static ProjectOverlay()
        {
            EditorApplication.projectWindowItemOnGUI += OnProjectWindowItemGUI;
        }

        private static GUIStyle GetBadgeStyle()
        {
            if (_badgeStyle == null)
            {
                _badgeStyle = new GUIStyle(EditorStyles.miniBoldLabel)
                {
                    alignment = TextAnchor.MiddleRight,
                    fontSize = 10
                };
            }
            return _badgeStyle;
        }

        private static void OnProjectWindowItemGUI(string guid, Rect selectionRect)
        {
            if (string.IsNullOrEmpty(guid) || selectionRect.height > 24f)
                return;

            string path = AssetDatabase.GUIDToAssetPath(guid);
            if (string.IsNullOrEmpty(path))
                return;

            string badge = null;
            Color color = Color.white;

            if (ChibiWorkspace.TryGetLock(path, out var lockInfo))
            {
                bool mine = string.Equals(lockInfo.owner, ChibiWorkspace.Config.username, StringComparison.OrdinalIgnoreCase);
                badge = mine ? "● LOCKED (YOU)" : $"● LOCKED ({lockInfo.owner})";
                color = mine ? new Color(0.35f, 0.95f, 0.45f) : new Color(1f, 0.35f, 0.35f);
            }
            else if (ChibiWorkspace.PendingStatusByPath.TryGetValue(path, out string kind))
            {
                if (kind == "added")
                {
                    badge = "+ ADDED";
                    color = new Color(0.45f, 0.95f, 0.55f);
                }
                else if (kind == "modified")
                {
                    badge = "● MODIFIED";
                    color = new Color(0.45f, 0.75f, 1f);
                }
            }

            if (badge != null)
            {
                var prev = GUI.color;
                GUI.color = color;
                var rect = new Rect(selectionRect.xMax - 125f, selectionRect.y, 120f, selectionRect.height);
                GUI.Label(rect, badge, GetBadgeStyle());
                GUI.color = prev;
            }
        }
    }
}
