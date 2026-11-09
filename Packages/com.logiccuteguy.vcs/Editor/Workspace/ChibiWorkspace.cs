using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using LogicCuteGuy.Vcs.Editor.Client;
using UnityEditor;
using UnityEngine;

namespace LogicCuteGuy.Vcs.Editor.Workspace
{
    [Serializable]
    public class WorkspaceConfig
    {
        public string serverUrl = "http://100.65.127.80:8087";
        public string repo = "vrc-world";
        public string branch = "main";
        public long baseChangesetId = 0;
        public string username = "";
        public string token = "";
        public List<FileEntry> files = new List<FileEntry>();
    }

    public class PendingChange
    {
        public string Path;
        public string Kind; // "added" | "modified" | "deleted"
        public string Sha256;
        public ulong Size;
        public bool Selected = true;
    }

    public static class ChibiWorkspace
    {
        private struct CachedHash
        {
            public long LastWriteTicks;
            public long Length;
            public string Sha256;
        }

        private static readonly Dictionary<string, CachedHash> HashCache = new Dictionary<string, CachedHash>(StringComparer.OrdinalIgnoreCase);

        public static WorkspaceConfig Config { get; private set; } = new WorkspaceConfig();
        public static bool IsInitialized => File.Exists(WorkspaceFilePath) && !string.IsNullOrEmpty(Config.token);
        public static bool AutoCheckoutEnabled
        {
            get => EditorPrefs.GetBool("ChibiVcs.AutoCheckout", true);
            set => EditorPrefs.SetBool("ChibiVcs.AutoCheckout", value);
        }

        public static List<PendingChange> PendingChanges { get; } = new List<PendingChange>();
        public static Dictionary<string, string> PendingStatusByPath { get; } = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        public static List<LockInfo> ActiveLocks { get; private set; } = new List<LockInfo>();
        public static Dictionary<string, LockInfo> LocksByPath { get; } = new Dictionary<string, LockInfo>(StringComparer.OrdinalIgnoreCase);
        public static List<ChangesetSummary> History { get; private set; } = new List<ChangesetSummary>();
        public static List<ChangesetSummary> IncomingChangesets { get; private set; } = new List<ChangesetSummary>();
        public static List<BranchInfo> Branches { get; private set; } = new List<BranchInfo>();

        public static string ProjectRoot => Path.GetFullPath(Path.Combine(Application.dataPath, ".."));
        public static string WorkspaceDir => Path.Combine(ProjectRoot, ".chibivcs");
        public static string WorkspaceFilePath => Path.Combine(WorkspaceDir, "workspace.json");

        static ChibiWorkspace()
        {
            LoadConfig();
        }

        public static void LoadConfig()
        {
            try
            {
                if (File.Exists(WorkspaceFilePath))
                {
                    string json = File.ReadAllText(WorkspaceFilePath);
                    var loaded = JsonUtility.FromJson<WorkspaceConfig>(json);
                    if (loaded != null)
                        Config = loaded;
                }
            }
            catch (Exception ex)
            {
                Debug.LogWarning($"[Chibi VCS] Failed to read workspace.json: {ex.Message}");
            }
        }

        public static void SaveConfig()
        {
            try
            {
                Directory.CreateDirectory(WorkspaceDir);
                string json = JsonUtility.ToJson(Config, true);
                File.WriteAllText(WorkspaceFilePath, json);
            }
            catch (Exception ex)
            {
                Debug.LogError($"[Chibi VCS] Failed to save workspace.json: {ex.Message}");
            }
        }

        public static bool TryGetLock(string normPath, out LockInfo lockInfo)
        {
            if (string.IsNullOrEmpty(normPath))
            {
                lockInfo = null;
                return false;
            }
            return LocksByPath.TryGetValue(normPath.Replace('\\', '/'), out lockInfo);
        }

        public static bool IsLockedByOther(string normPath, out string owner)
        {
            owner = null;
            if (TryGetLock(normPath, out var info))
            {
                if (!string.Equals(info.owner, Config.username, StringComparison.OrdinalIgnoreCase))
                {
                    owner = info.owner;
                    return true;
                }
            }
            return false;
        }

        public static bool IsLockedByMe(string normPath)
        {
            if (TryGetLock(normPath, out var info))
            {
                return string.Equals(info.owner, Config.username, StringComparison.OrdinalIgnoreCase);
            }
            return false;
        }

        /// <summary>
        /// Injects or clears a simulated lock entry (used by automated tests & verification).
        /// </summary>
        public static void SetLockOverrideForTest(string normPath, string owner)
        {
            string clean = IgnoreRules.NormalizeRelPath(normPath);
            if (string.IsNullOrEmpty(clean)) return;
            if (string.IsNullOrEmpty(owner))
            {
                LocksByPath.Remove(clean);
                ActiveLocks.RemoveAll(l => string.Equals(l.path, clean, StringComparison.OrdinalIgnoreCase));
            }
            else
            {
                var li = new LockInfo
                {
                    path = clean,
                    branch = Config.branch,
                    owner = owner,
                    locked_at_unix = DateTimeOffset.UtcNow.ToUnixTimeSeconds()
                };
                LocksByPath[clean] = li;
                ActiveLocks.RemoveAll(l => string.Equals(l.path, clean, StringComparison.OrdinalIgnoreCase));
                ActiveLocks.Add(li);
            }
        }

        private static string GetFileSha256Cached(string fullPath, string normPath)
        {
            var fi = new FileInfo(fullPath);
            long ticks = fi.LastWriteTimeUtc.Ticks;
            long len = fi.Length;
            if (HashCache.TryGetValue(normPath, out var cached) && cached.LastWriteTicks == ticks && cached.Length == len)
            {
                return cached.Sha256;
            }

            byte[] bytes = File.ReadAllBytes(fullPath);
            string sha = IgnoreRules.ComputeSha256Hex(bytes);
            HashCache[normPath] = new CachedHash
            {
                LastWriteTicks = ticks,
                Length = len,
                Sha256 = sha
            };
            return sha;
        }

        private static void ScanDirectory(string root, string currentDir, Dictionary<string, (string sha, ulong size)> outFiles)
        {
            foreach (var dir in Directory.GetDirectories(currentDir))
            {
                string relDir = Path.GetRelativePath(root, dir);
                string normDir = IgnoreRules.NormalizeRelPath(relDir);
                if (normDir == null || IgnoreRules.IsIgnored(normDir))
                    continue;
                ScanDirectory(root, dir, outFiles);
            }

            foreach (var file in Directory.GetFiles(currentDir))
            {
                string relFile = Path.GetRelativePath(root, file);
                string normFile = IgnoreRules.NormalizeRelPath(relFile);
                if (normFile == null || IgnoreRules.IsIgnored(normFile))
                    continue;
                var fi = new FileInfo(file);
                string sha = GetFileSha256Cached(file, normFile);
                outFiles[normFile] = (sha, (ulong)fi.Length);
            }
        }

        public static Task RefreshLocalPendingAsync()
        {
            string root = ProjectRoot;
            var baseFiles = Config.files != null ? Config.files.ToList() : new List<FileEntry>();
            var prevSelections = PendingChanges.ToDictionary(p => p.Path, p => p.Selected, StringComparer.OrdinalIgnoreCase);

            return Task.Run(() =>
            {
                var disk = new Dictionary<string, (string sha, ulong size)>(StringComparer.OrdinalIgnoreCase);
                ScanDirectory(root, root, disk);

                var baseMap = new Dictionary<string, FileEntry>(StringComparer.OrdinalIgnoreCase);
                foreach (var f in baseFiles)
                {
                    baseMap[f.path] = f;
                }

                var list = new List<PendingChange>();
                foreach (var kv in disk)
                {
                    if (!baseMap.TryGetValue(kv.Key, out var bf))
                    {
                        list.Add(new PendingChange
                        {
                            Path = kv.Key,
                            Kind = "added",
                            Sha256 = kv.Value.sha,
                            Size = kv.Value.size,
                            Selected = !prevSelections.TryGetValue(kv.Key, out bool sel) || sel
                        });
                    }
                    else if (!string.Equals(bf.sha256, kv.Value.sha, StringComparison.OrdinalIgnoreCase))
                    {
                        list.Add(new PendingChange
                        {
                            Path = kv.Key,
                            Kind = "modified",
                            Sha256 = kv.Value.sha,
                            Size = kv.Value.size,
                            Selected = !prevSelections.TryGetValue(kv.Key, out bool sel) || sel
                        });
                    }
                }

                foreach (var kv in baseMap)
                {
                    if (!disk.ContainsKey(kv.Key))
                    {
                        list.Add(new PendingChange
                        {
                            Path = kv.Key,
                            Kind = "deleted",
                            Sha256 = "",
                            Size = 0,
                            Selected = !prevSelections.TryGetValue(kv.Key, out bool sel) || sel
                        });
                    }
                }

                list.Sort((a, b) => string.CompareOrdinal(a.Path, b.Path));

                PendingChanges.Clear();
                PendingChanges.AddRange(list);
                PendingStatusByPath.Clear();
                foreach (var item in list)
                {
                    PendingStatusByPath[item.Path] = item.Kind;
                }
            });
        }

        public static async Task RefreshRemoteStateAsync()
        {
            LoadConfig();
            if (!IsInitialized)
                return;

            var locksTask = ApiClient.ListLocksAsync(Config.serverUrl, Config.token, Config.repo, Config.branch);
            var csTask = ApiClient.ListChangesetsAsync(Config.serverUrl, Config.token, Config.repo, Config.branch, 50);
            var branchesTask = ApiClient.ListBranchesAsync(Config.serverUrl, Config.token, Config.repo);

            await Task.WhenAll(locksTask, csTask, branchesTask);

            ActiveLocks = locksTask.Result;
            LocksByPath.Clear();
            foreach (var l in ActiveLocks)
            {
                LocksByPath[l.path] = l;
            }

            History = csTask.Result;
            IncomingChangesets = History.Where(c => c.id > Config.baseChangesetId).OrderBy(c => c.id).ToList();
            Branches = branchesTask.Result;
        }

        public static async Task LoginAndConnectAsync(string serverUrl, string repo, string branch, string username, string password)
        {
            var login = await ApiClient.LoginAsync(serverUrl, username, password);
            Config.serverUrl = serverUrl.Trim().TrimEnd('/');
            Config.repo = repo.Trim();
            Config.branch = string.IsNullOrWhiteSpace(branch) ? "main" : branch.Trim();
            Config.username = login.username;
            Config.token = login.token;
            SaveConfig();

            await RefreshRemoteStateAsync();
            await RefreshLocalPendingAsync();
        }

        public static async Task<(long changesetId, bool rebased)> CheckinSelectedAsync(string comment)
        {
            if (string.IsNullOrWhiteSpace(comment))
                throw new InvalidOperationException("Please enter a checkin comment.");

            // Auto-include companion .meta files when an asset is selected
            var selectedSet = new HashSet<string>(
                PendingChanges.Where(p => p.Selected).Select(p => p.Path),
                StringComparer.OrdinalIgnoreCase
            );
            foreach (var p in PendingChanges)
            {
                if (selectedSet.Contains(p.Path) && !p.Path.EndsWith(".meta", StringComparison.OrdinalIgnoreCase))
                {
                    selectedSet.Add(p.Path + ".meta");
                }
            }

            var toCommit = PendingChanges.Where(p => selectedSet.Contains(p.Path)).ToList();
            if (toCommit.Count == 0)
                throw new InvalidOperationException("No files selected for checkin.");

            var uploadHashes = toCommit
                .Where(c => c.Kind != "deleted")
                .Select(c => c.Sha256)
                .Distinct(StringComparer.OrdinalIgnoreCase)
                .ToList();

            if (uploadHashes.Count > 0)
            {
                var missing = await ApiClient.GetMissingBlobsAsync(Config.serverUrl, Config.token, Config.repo, uploadHashes);
                var missingSet = new HashSet<string>(missing, StringComparer.OrdinalIgnoreCase);

                string root = ProjectRoot;
                foreach (var item in toCommit)
                {
                    if (item.Kind != "deleted" && missingSet.Contains(item.Sha256))
                    {
                        byte[] bytes = await File.ReadAllBytesAsync(Path.Combine(root, item.Path));
                        await ApiClient.UploadBlobAsync(Config.serverUrl, Config.token, Config.repo, item.Sha256, bytes);
                        missingSet.Remove(item.Sha256);
                    }
                }
            }

            var req = new CheckinRequest
            {
                branch = Config.branch,
                parent_changeset_id = Config.baseChangesetId,
                comment = comment.Trim(),
                changes = toCommit.Select(c => new FileChangeItem
                {
                    path = c.Path,
                    kind = c.Kind,
                    sha256 = c.Sha256,
                    size = c.Size
                }).ToList()
            };

            var resp = await ApiClient.CheckinAsync(Config.serverUrl, Config.token, Config.repo, req);

            // Sync local base state to server tree at the new changeset
            var tree = await ApiClient.GetLatestTreeAsync(Config.serverUrl, Config.token, Config.repo, Config.branch);
            Config.baseChangesetId = tree.changeset_id;
            Config.files = tree.files;
            SaveConfig();

            await RefreshRemoteStateAsync();
            await RefreshLocalPendingAsync();
            return (resp.changeset_id, resp.rebased);
        }

        public static async Task<long> UpdateWorkspaceAsync()
        {
            var tree = await ApiClient.GetLatestTreeAsync(Config.serverUrl, Config.token, Config.repo, Config.branch);
            string root = ProjectRoot;

            var remotePaths = new HashSet<string>(tree.files.Select(f => f.path), StringComparer.OrdinalIgnoreCase);
            var filesToDelete = new List<string>();
            if (Config.files != null)
            {
                foreach (var old in Config.files)
                {
                    if (!remotePaths.Contains(old.path))
                    {
                        filesToDelete.Add(Path.Combine(root, old.path));
                    }
                }
            }

            var downloads = new List<(string fullPath, string normPath, byte[] bytes)>();
            foreach (var f in tree.files)
            {
                string fullPath = Path.Combine(root, f.path);
                bool needsDownload = true;
                if (File.Exists(fullPath))
                {
                    string localSha = GetFileSha256Cached(fullPath, f.path);
                    if (string.Equals(localSha, f.sha256, StringComparison.OrdinalIgnoreCase))
                        needsDownload = false;
                }

                if (needsDownload)
                {
                    byte[] data = await ApiClient.DownloadBlobAsync(Config.serverUrl, Config.token, Config.repo, f.sha256);
                    downloads.Add((fullPath, f.path, data));
                }
            }

            AssetDatabase.StartAssetEditing();
            try
            {
                foreach (var del in filesToDelete)
                {
                    if (File.Exists(del))
                        File.Delete(del);
                }

                foreach (var dl in downloads)
                {
                    string dir = Path.GetDirectoryName(dl.fullPath);
                    if (!string.IsNullOrEmpty(dir))
                        Directory.CreateDirectory(dir);
                    File.WriteAllBytes(dl.fullPath, dl.bytes);
                }
            }
            finally
            {
                AssetDatabase.StopAssetEditing();
            }

            Config.baseChangesetId = tree.changeset_id;
            Config.files = tree.files;
            SaveConfig();

            AssetDatabase.Refresh();
            await RefreshRemoteStateAsync();
            await RefreshLocalPendingAsync();
            return tree.changeset_id;
        }
    }
}
