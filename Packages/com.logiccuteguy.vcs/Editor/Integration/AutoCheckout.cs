using System;
using System.Collections.Generic;
using LogicCuteGuy.Vcs.Editor.Client;
using LogicCuteGuy.Vcs.Editor.Workspace;
using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;
using UnityEngine.SceneManagement;

namespace LogicCuteGuy.Vcs.Editor.Integration
{
    [InitializeOnLoad]
    public class AutoCheckout : AssetModificationProcessor
    {
        static AutoCheckout()
        {
            EditorSceneManager.sceneOpened += OnSceneOpened;
        }

        private static async void OnSceneOpened(Scene scene, OpenSceneMode mode)
        {
            if (string.IsNullOrEmpty(scene.path) || !ChibiWorkspace.IsInitialized)
                return;

            try
            {
                await ChibiWorkspace.RefreshRemoteStateAsync();
                if (ChibiWorkspace.IsLockedByOther(scene.path, out string owner))
                {
                    Debug.LogWarning($"[Chibi VCS] Active scene '{scene.path}' is currently locked by '{owner}'. Saving is blocked until unlocked.");
                }
            }
            catch
            {
                // Ignore transient offline errors on scene open
            }
        }

        /// <summary>
        /// Checks whether an asset path is allowed to be saved locally.
        /// Returns false if locked on the server by another team member.
        /// </summary>
        public static bool CanSaveAsset(string assetPath, out string lockedByOwner)
        {
            lockedByOwner = null;
            string norm = IgnoreRules.NormalizeRelPath(assetPath);
            if (string.IsNullOrEmpty(norm))
                return true;

            if (ChibiWorkspace.IsLockedByOther(norm, out lockedByOwner))
                return false;

            return true;
        }

        private static string[] OnWillSaveAssets(string[] paths)
        {
            if (paths == null || paths.Length == 0)
                return paths;

            var allowed = new List<string>(paths.Length);
            var toAutoLock = new List<string>();

            foreach (var p in paths)
            {
                string norm = IgnoreRules.NormalizeRelPath(p);
                if (string.IsNullOrEmpty(norm))
                {
                    allowed.Add(p);
                    continue;
                }

                if (!CanSaveAsset(norm, out string holder))
                {
                    Debug.LogError($"[Chibi VCS] Save blocked! '{norm}' is exclusively locked by teammate '{holder}'.");
                    if (!Application.isBatchMode)
                    {
                        EditorUtility.DisplayDialog(
                            "Chibi VCS — Asset Locked by Teammate",
                            $"Cannot save '{norm}' because it is currently locked by '{holder}'.\n\nCoordinate with {holder} or ask an admin to release the lock.",
                            "OK"
                        );
                    }
                    continue;
                }

                allowed.Add(p);

                if (ChibiWorkspace.IsInitialized &&
                    ChibiWorkspace.AutoCheckoutEnabled &&
                    IgnoreRules.IsLockableAsset(norm) &&
                    !ChibiWorkspace.IsLockedByMe(norm))
                {
                    toAutoLock.Add(norm);
                }
            }

            if (toAutoLock.Count > 0)
            {
                AutoLockAsync(toAutoLock);
            }

            return allowed.ToArray();
        }

        private static async void AutoLockAsync(List<string> normPaths)
        {
            try
            {
                await ApiClient.AcquireLocksAsync(
                    ChibiWorkspace.Config.serverUrl,
                    ChibiWorkspace.Config.token,
                    ChibiWorkspace.Config.repo,
                    ChibiWorkspace.Config.branch,
                    normPaths
                );
                await ChibiWorkspace.RefreshRemoteStateAsync();
                EditorApplication.RepaintProjectWindow();
            }
            catch (Exception ex)
            {
                Debug.LogWarning($"[Chibi VCS] Auto-checkout lock failed: {ex.Message}");
            }
        }
    }
}
