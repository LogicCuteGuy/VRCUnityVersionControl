using System;
using System.Collections.Generic;
using System.Linq;
using LogicCuteGuy.Vcs.Editor.Client;
using LogicCuteGuy.Vcs.Editor.Workspace;
using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;

namespace LogicCuteGuy.Vcs.Editor.UI
{
    public class ChibiVcsWindow : EditorWindow
    {
        private enum Tab
        {
            PendingChanges,
            Incoming,
            Changesets,
            Branches,
            Locks,
            WorkspaceLogin
        }

        private Tab _tab = Tab.PendingChanges;
        private string _comment = "";
        private string _newBranchName = "";
        private string _loginServerUrl = "http://100.65.127.80:8087";
        private string _loginRepo = "vrc-world";
        private string _loginBranch = "main";
        private string _loginUser = "LogicCuteGuy";
        private string _loginPassword = "";

        private Vector2 _pendingScroll;
        private Vector2 _incomingScroll;
        private Vector2 _historyScroll;
        private Vector2 _branchesScroll;
        private Vector2 _locksScroll;

        private bool _busy;
        private string _busyMessage = "";
        private string _statusBanner = "";
        private bool _statusSuccess = true;

        [MenuItem("Window/LogicCuteGuy VCS", false, 20)]
        [MenuItem("Tools/LogicCuteGuy VCS", false, 1)]
        public static ChibiVcsWindow OpenWindow()
        {
            var w = GetWindow<ChibiVcsWindow>("LogicCuteGuy VCS");
            w.minSize = new Vector2(560, 440);
            w.Show();
            return w;
        }

        [MenuItem("Assets/LogicCuteGuy VCS/Lock Selected Asset", false, 30)]
        private static async void ContextLockSelected()
        {
            if (!ChibiWorkspace.IsInitialized || Selection.activeObject == null) return;
            string path = IgnoreRules.NormalizeRelPath(AssetDatabase.GetAssetPath(Selection.activeObject));
            if (string.IsNullOrEmpty(path)) return;
            try
            {
                await ApiClient.AcquireLocksAsync(
                    ChibiWorkspace.Config.serverUrl,
                    ChibiWorkspace.Config.token,
                    ChibiWorkspace.Config.repo,
                    ChibiWorkspace.Config.branch,
                    new List<string> { path }
                );
                await ChibiWorkspace.RefreshRemoteStateAsync();
                EditorApplication.RepaintProjectWindow();
                Debug.Log($"[LogicCuteGuy VCS] Locked '{path}'");
            }
            catch (Exception ex)
            {
                Debug.LogError($"[LogicCuteGuy VCS] Lock failed: {ex.Message}");
            }
        }

        [MenuItem("Assets/LogicCuteGuy VCS/Unlock Selected Asset", false, 31)]
        private static async void ContextUnlockSelected()
        {
            if (!ChibiWorkspace.IsInitialized || Selection.activeObject == null) return;
            string path = IgnoreRules.NormalizeRelPath(AssetDatabase.GetAssetPath(Selection.activeObject));
            if (string.IsNullOrEmpty(path)) return;
            try
            {
                await ApiClient.ReleaseLocksAsync(
                    ChibiWorkspace.Config.serverUrl,
                    ChibiWorkspace.Config.token,
                    ChibiWorkspace.Config.repo,
                    ChibiWorkspace.Config.branch,
                    new List<string> { path },
                    false
                );
                await ChibiWorkspace.RefreshRemoteStateAsync();
                EditorApplication.RepaintProjectWindow();
                Debug.Log($"[LogicCuteGuy VCS] Unlocked '{path}'");
            }
            catch (Exception ex)
            {
                Debug.LogError($"[LogicCuteGuy VCS] Unlock failed: {ex.Message}");
            }
        }

        private void OnEnable()
        {
            ChibiWorkspace.LoadConfig();
            _loginServerUrl = ChibiWorkspace.Config.serverUrl;
            _loginRepo = ChibiWorkspace.Config.repo;
            _loginBranch = ChibiWorkspace.Config.branch;
            if (!string.IsNullOrEmpty(ChibiWorkspace.Config.username))
                _loginUser = ChibiWorkspace.Config.username;

            if (!ChibiWorkspace.IsInitialized)
                _tab = Tab.WorkspaceLogin;

            RefreshAll();
        }

        public async void RefreshAll()
        {
            if (_busy) return;
            _busy = true;
            _busyMessage = "Refreshing workspace & server state...";
            Repaint();
            try
            {
                await ChibiWorkspace.RefreshLocalPendingAsync();
                if (ChibiWorkspace.IsInitialized)
                {
                    await ChibiWorkspace.RefreshRemoteStateAsync();
                }
                EditorApplication.RepaintProjectWindow();
            }
            catch (Exception ex)
            {
                SetBanner(false, ex.Message);
            }
            finally
            {
                _busy = false;
                Repaint();
            }
        }

        private void SetBanner(bool ok, string msg)
        {
            _statusSuccess = ok;
            _statusBanner = msg;
            Repaint();
        }

        private void OnGUI()
        {
            DrawHeader();
            DrawTabBar();

            if (!string.IsNullOrEmpty(_statusBanner))
            {
                EditorGUILayout.BeginHorizontal(EditorStyles.helpBox);
                EditorGUILayout.LabelField((_statusSuccess ? "✔ " : "✖ ") + _statusBanner, EditorStyles.wordWrappedLabel);
                if (GUILayout.Button("✕", GUILayout.Width(24)))
                    _statusBanner = "";
                EditorGUILayout.EndHorizontal();
            }

            if (_busy)
            {
                EditorGUILayout.HelpBox("⏳ " + _busyMessage, MessageType.Info);
            }

            switch (_tab)
            {
                case Tab.PendingChanges:
                    DrawPendingTab();
                    break;
                case Tab.Incoming:
                    DrawIncomingTab();
                    break;
                case Tab.Changesets:
                    DrawHistoryTab();
                    break;
                case Tab.Branches:
                    DrawBranchesTab();
                    break;
                case Tab.Locks:
                    DrawLocksTab();
                    break;
                case Tab.WorkspaceLogin:
                    DrawLoginTab();
                    break;
            }
        }

        private void DrawHeader()
        {
            EditorGUILayout.BeginVertical(EditorStyles.helpBox);
            EditorGUILayout.BeginHorizontal();

            bool connected = ChibiWorkspace.IsInitialized;
            GUI.color = connected ? new Color(0.4f, 1f, 0.45f) : new Color(1f, 0.55f, 0.4f);
            GUILayout.Label(
                connected
                    ? $"● {ChibiWorkspace.Config.repo}@{ChibiWorkspace.Config.branch} (cs:{ChibiWorkspace.Config.baseChangesetId})"
                    : "● Not Connected",
                EditorStyles.boldLabel
            );
            GUI.color = Color.white;

            if (connected)
            {
                GUILayout.Label($"User: {ChibiWorkspace.Config.username}", EditorStyles.miniLabel);
            }

            GUILayout.FlexibleSpace();

            GUI.enabled = !_busy;
            if (GUILayout.Button("🔄 Refresh", GUILayout.Width(84)))
                RefreshAll();
            GUI.enabled = true;
            EditorGUILayout.EndHorizontal();

            if (connected)
            {
                EditorGUILayout.BeginHorizontal();
                GUI.enabled = !_busy;

                int incomingCount = ChibiWorkspace.IncomingChangesets.Count;
                GUI.backgroundColor = incomingCount > 0 ? new Color(1f, 0.85f, 0.35f) : new Color(0.35f, 0.75f, 1f);
                string updateLabel = incomingCount > 0
                    ? $"⬇ Update Workspace ({incomingCount} incoming)"
                    : "⬇ Update Workspace";
                if (GUILayout.Button(updateLabel, GUILayout.Height(24)))
                {
                    RunUpdateWorkspace();
                }

                string scenePath = IgnoreRules.NormalizeRelPath(EditorSceneManager.GetActiveScene().path);
                if (!string.IsNullOrEmpty(scenePath))
                {
                    if (ChibiWorkspace.TryGetLock(scenePath, out var sceneLock))
                    {
                        bool mine = string.Equals(sceneLock.owner, ChibiWorkspace.Config.username, StringComparison.OrdinalIgnoreCase);
                        GUI.backgroundColor = mine ? new Color(0.45f, 0.95f, 0.5f) : new Color(1f, 0.4f, 0.4f);
                        string lockBtn = mine ? "🔓 Unlock Active Scene (You)" : $"🔒 Scene Locked by {sceneLock.owner}";
                        if (GUILayout.Button(lockBtn, GUILayout.Height(24)) && mine)
                        {
                            RunUnlockPaths(new List<string> { scenePath }, false);
                        }
                    }
                    else
                    {
                        GUI.backgroundColor = new Color(0.9f, 0.9f, 0.9f);
                        if (GUILayout.Button("🔒 Lock Active Scene", GUILayout.Height(24)))
                        {
                            RunLockPaths(new List<string> { scenePath });
                        }
                    }
                }

                GUI.backgroundColor = Color.white;
                GUI.enabled = true;
                EditorGUILayout.EndHorizontal();
            }

            EditorGUILayout.EndVertical();
        }

        private void DrawTabBar()
        {
            EditorGUILayout.BeginHorizontal(EditorStyles.toolbar);
            if (GUILayout.Toggle(_tab == Tab.PendingChanges, $"Pending ({ChibiWorkspace.PendingChanges.Count})", EditorStyles.toolbarButton))
                _tab = Tab.PendingChanges;
            if (GUILayout.Toggle(_tab == Tab.Incoming, $"Incoming ({ChibiWorkspace.IncomingChangesets.Count})", EditorStyles.toolbarButton))
                _tab = Tab.Incoming;
            if (GUILayout.Toggle(_tab == Tab.Changesets, $"Changesets ({ChibiWorkspace.History.Count})", EditorStyles.toolbarButton))
                _tab = Tab.Changesets;
            if (GUILayout.Toggle(_tab == Tab.Branches, $"Branches ({ChibiWorkspace.Branches.Count})", EditorStyles.toolbarButton))
                _tab = Tab.Branches;
            if (GUILayout.Toggle(_tab == Tab.Locks, $"Locks ({ChibiWorkspace.ActiveLocks.Count})", EditorStyles.toolbarButton))
                _tab = Tab.Locks;
            if (GUILayout.Toggle(_tab == Tab.WorkspaceLogin, "Workspace", EditorStyles.toolbarButton))
                _tab = Tab.WorkspaceLogin;
            EditorGUILayout.EndHorizontal();
        }

        private void DrawPendingTab()
        {
            EditorGUILayout.BeginHorizontal();
            if (GUILayout.Button("Select All", GUILayout.Width(80)))
                ChibiWorkspace.PendingChanges.ForEach(p => p.Selected = true);
            if (GUILayout.Button("None", GUILayout.Width(60)))
                ChibiWorkspace.PendingChanges.ForEach(p => p.Selected = false);
            GUILayout.FlexibleSpace();
            int selCount = ChibiWorkspace.PendingChanges.Count(p => p.Selected);
            GUILayout.Label($"{selCount} / {ChibiWorkspace.PendingChanges.Count} selected", EditorStyles.miniLabel);
            EditorGUILayout.EndHorizontal();

            _pendingScroll = EditorGUILayout.BeginScrollView(_pendingScroll, GUILayout.ExpandHeight(true));
            if (ChibiWorkspace.PendingChanges.Count == 0)
            {
                EditorGUILayout.HelpBox("No local changes detected.", MessageType.Info);
            }
            else
            {
                foreach (var ch in ChibiWorkspace.PendingChanges)
                {
                    EditorGUILayout.BeginHorizontal();
                    ch.Selected = EditorGUILayout.Toggle(ch.Selected, GUILayout.Width(18));
                    GUILayout.Label($"[{ch.Kind.ToUpperInvariant()[0]}]", EditorStyles.boldLabel, GUILayout.Width(28));
                    GUILayout.Label(ch.Path);
                    EditorGUILayout.EndHorizontal();
                }
            }
            EditorGUILayout.EndScrollView();

            EditorGUILayout.BeginVertical(EditorStyles.helpBox);
            GUILayout.Label("Checkin Comment:", EditorStyles.boldLabel);
            _comment = EditorGUILayout.TextArea(_comment, GUILayout.Height(46));

            GUI.enabled = !_busy && ChibiWorkspace.IsInitialized && selCount > 0 && !string.IsNullOrWhiteSpace(_comment);
            GUI.backgroundColor = new Color(0.35f, 0.9f, 0.45f);
            if (GUILayout.Button($"✔ Checkin Selected ({selCount} items)", GUILayout.Height(30)))
            {
                RunCheckin();
            }
            GUI.backgroundColor = Color.white;
            GUI.enabled = true;
            EditorGUILayout.EndVertical();
        }

        private void DrawIncomingTab()
        {
            _incomingScroll = EditorGUILayout.BeginScrollView(_incomingScroll, GUILayout.ExpandHeight(true));
            if (ChibiWorkspace.IncomingChangesets.Count == 0)
            {
                EditorGUILayout.HelpBox("Your workspace is up to date with the server branch.", MessageType.Info);
            }
            else
            {
                foreach (var cs in ChibiWorkspace.IncomingChangesets)
                {
                    EditorGUILayout.BeginVertical(EditorStyles.helpBox);
                    EditorGUILayout.LabelField($"cs:{cs.id} by {cs.author} ({cs.change_count} files)", EditorStyles.boldLabel);
                    EditorGUILayout.LabelField(cs.comment, EditorStyles.wordWrappedLabel);
                    EditorGUILayout.EndVertical();
                }
            }
            EditorGUILayout.EndScrollView();
        }

        private void DrawHistoryTab()
        {
            _historyScroll = EditorGUILayout.BeginScrollView(_historyScroll, GUILayout.ExpandHeight(true));
            foreach (var cs in ChibiWorkspace.History)
            {
                EditorGUILayout.BeginVertical(EditorStyles.helpBox);
                string marker = cs.id == ChibiWorkspace.Config.baseChangesetId ? " (Current Workspace)" : "";
                EditorGUILayout.LabelField($"cs:{cs.id}{marker} — {cs.author} [{cs.branch}] ({cs.change_count} files)", EditorStyles.boldLabel);
                EditorGUILayout.LabelField(cs.comment, EditorStyles.wordWrappedLabel);
                EditorGUILayout.EndVertical();
            }
            EditorGUILayout.EndScrollView();
        }

        private void DrawBranchesTab()
        {
            _branchesScroll = EditorGUILayout.BeginScrollView(_branchesScroll, GUILayout.ExpandHeight(true));
            foreach (var b in ChibiWorkspace.Branches)
            {
                EditorGUILayout.BeginHorizontal(EditorStyles.helpBox);
                bool active = string.Equals(b.name, ChibiWorkspace.Config.branch, StringComparison.OrdinalIgnoreCase);
                GUILayout.Label(active ? $"★ {b.name} (head cs:{b.head_changeset_id})" : $"{b.name} (head cs:{b.head_changeset_id})", EditorStyles.boldLabel);
                GUILayout.FlexibleSpace();
                if (!active && GUILayout.Button("Switch", GUILayout.Width(70)))
                {
                    ChibiWorkspace.Config.branch = b.name;
                    ChibiWorkspace.SaveConfig();
                    RunUpdateWorkspace();
                }
                EditorGUILayout.EndHorizontal();
            }
            EditorGUILayout.EndScrollView();

            EditorGUILayout.BeginHorizontal(EditorStyles.helpBox);
            _newBranchName = EditorGUILayout.TextField("New Branch Name:", _newBranchName);
            if (GUILayout.Button("Create Branch", GUILayout.Width(110)) && !string.IsNullOrWhiteSpace(_newBranchName))
            {
                RunCreateBranch(_newBranchName.Trim());
            }
            EditorGUILayout.EndHorizontal();
        }

        private void DrawLocksTab()
        {
            _locksScroll = EditorGUILayout.BeginScrollView(_locksScroll, GUILayout.ExpandHeight(true));
            if (ChibiWorkspace.ActiveLocks.Count == 0)
            {
                EditorGUILayout.HelpBox("No active locks on this branch.", MessageType.Info);
            }
            else
            {
                foreach (var l in ChibiWorkspace.ActiveLocks)
                {
                    EditorGUILayout.BeginHorizontal(EditorStyles.helpBox);
                    bool mine = string.Equals(l.owner, ChibiWorkspace.Config.username, StringComparison.OrdinalIgnoreCase);
                    GUILayout.Label(mine ? $"🟢 {l.owner} (You)" : $"🔴 {l.owner}", EditorStyles.boldLabel, GUILayout.Width(140));
                    GUILayout.Label(l.path, EditorStyles.wordWrappedLabel);
                    if (GUILayout.Button(mine ? "Unlock" : "Force Unlock", GUILayout.Width(95)))
                    {
                        RunUnlockPaths(new List<string> { l.path }, !mine);
                    }
                    EditorGUILayout.EndHorizontal();
                }
            }
            EditorGUILayout.EndScrollView();
        }

        private void DrawLoginTab()
        {
            EditorGUILayout.Space(8);
            GUILayout.Label("LogicCuteGuy VCS Server & Workspace Connection", EditorStyles.boldLabel);
            _loginServerUrl = EditorGUILayout.TextField("Server URL:", _loginServerUrl);
            _loginRepo = EditorGUILayout.TextField("Repository:", _loginRepo);
            _loginBranch = EditorGUILayout.TextField("Branch:", _loginBranch);
            _loginUser = EditorGUILayout.TextField("Username:", _loginUser);
            _loginPassword = EditorGUILayout.PasswordField("Password:", _loginPassword);

            EditorGUILayout.Space(6);
            ChibiWorkspace.AutoCheckoutEnabled = EditorGUILayout.Toggle("Auto-Lock on Asset Save", ChibiWorkspace.AutoCheckoutEnabled);

            EditorGUILayout.Space(10);
            GUI.enabled = !_busy;
            if (GUILayout.Button("🔑 Login & Connect Workspace", GUILayout.Height(32)))
            {
                RunLogin();
            }
            GUI.enabled = true;
        }

        private async void RunLogin()
        {
            _busy = true;
            _busyMessage = "Authenticating with ChibiVCS server...";
            Repaint();
            try
            {
                await ChibiWorkspace.LoginAndConnectAsync(_loginServerUrl, _loginRepo, _loginBranch, _loginUser, _loginPassword);
                _loginPassword = "";
                SetBanner(true, $"Connected as {ChibiWorkspace.Config.username} to {ChibiWorkspace.Config.repo}@{ChibiWorkspace.Config.branch}");
                _tab = Tab.PendingChanges;
            }
            catch (Exception ex)
            {
                SetBanner(false, $"Login failed: {ex.Message}");
            }
            finally
            {
                _busy = false;
                Repaint();
            }
        }

        private async void RunCheckin()
        {
            _busy = true;
            _busyMessage = "Uploading blobs & committing changeset...";
            Repaint();
            try
            {
                var (csId, rebased) = await ChibiWorkspace.CheckinSelectedAsync(_comment);
                _comment = "";
                SetBanner(true, $"Checked in cs:{csId}" + (rebased ? " (auto-rebased onto latest branch head)" : ""));
                EditorApplication.RepaintProjectWindow();
            }
            catch (Exception ex)
            {
                SetBanner(false, $"Checkin failed: {ex.Message}");
            }
            finally
            {
                _busy = false;
                Repaint();
            }
        }

        private async void RunUpdateWorkspace()
        {
            _busy = true;
            _busyMessage = "Downloading blobs & updating workspace...";
            Repaint();
            try
            {
                long csId = await ChibiWorkspace.UpdateWorkspaceAsync();
                SetBanner(true, $"Workspace updated to cs:{csId}");
                EditorApplication.RepaintProjectWindow();
            }
            catch (Exception ex)
            {
                SetBanner(false, $"Update failed: {ex.Message}");
            }
            finally
            {
                _busy = false;
                Repaint();
            }
        }

        private async void RunLockPaths(List<string> paths)
        {
            _busy = true;
            _busyMessage = "Acquiring lock...";
            Repaint();
            try
            {
                await ApiClient.AcquireLocksAsync(
                    ChibiWorkspace.Config.serverUrl,
                    ChibiWorkspace.Config.token,
                    ChibiWorkspace.Config.repo,
                    ChibiWorkspace.Config.branch,
                    paths
                );
                await ChibiWorkspace.RefreshRemoteStateAsync();
                EditorApplication.RepaintProjectWindow();
                SetBanner(true, $"Locked {string.Join(", ", paths)}");
            }
            catch (Exception ex)
            {
                SetBanner(false, $"Lock failed: {ex.Message}");
            }
            finally
            {
                _busy = false;
                Repaint();
            }
        }

        private async void RunUnlockPaths(List<string> paths, bool force)
        {
            _busy = true;
            _busyMessage = "Releasing lock...";
            Repaint();
            try
            {
                await ApiClient.ReleaseLocksAsync(
                    ChibiWorkspace.Config.serverUrl,
                    ChibiWorkspace.Config.token,
                    ChibiWorkspace.Config.repo,
                    ChibiWorkspace.Config.branch,
                    paths,
                    force
                );
                await ChibiWorkspace.RefreshRemoteStateAsync();
                EditorApplication.RepaintProjectWindow();
                SetBanner(true, $"Unlocked {string.Join(", ", paths)}");
            }
            catch (Exception ex)
            {
                SetBanner(false, $"Unlock failed: {ex.Message}");
            }
            finally
            {
                _busy = false;
                Repaint();
            }
        }

        private async void RunCreateBranch(string name)
        {
            _busy = true;
            _busyMessage = $"Creating branch '{name}'...";
            Repaint();
            try
            {
                await ApiClient.CreateBranchAsync(
                    ChibiWorkspace.Config.serverUrl,
                    ChibiWorkspace.Config.token,
                    ChibiWorkspace.Config.repo,
                    name,
                    ChibiWorkspace.Config.baseChangesetId
                );
                _newBranchName = "";
                await ChibiWorkspace.RefreshRemoteStateAsync();
                SetBanner(true, $"Created branch '{name}'");
            }
            catch (Exception ex)
            {
                SetBanner(false, $"Create branch failed: {ex.Message}");
            }
            finally
            {
                _busy = false;
                Repaint();
            }
        }
    }
}
