using System;
using System.Security.Cryptography;

namespace LogicCuteGuy.Vcs.Editor.Workspace
{
    public static class IgnoreRules
    {
        private static readonly string[] IgnoredRoots =
        {
            "library",
            "temp",
            "obj",
            "build",
            "builds",
            "logs",
            "usersettings",
            "memorycaptures",
            "recordings",
            ".git",
            ".vs",
            ".vscode",
            ".idea",
            ".chibivcs",
            "server"
        };

        private static readonly string[] LockableExtensions =
        {
            ".unity",
            ".prefab",
            ".asset",
            ".controller",
            ".anim",
            ".overridecontroller",
            ".mask",
            ".lighting",
            ".terrainlayer",
            ".mixer",
            ".fbx",
            ".blend",
            ".obj",
            ".dae",
            ".png",
            ".psd",
            ".tga",
            ".jpg",
            ".jpeg",
            ".exr",
            ".hdr",
            ".tif",
            ".tiff",
            ".wav",
            ".mp3",
            ".ogg",
            ".mp4",
            ".webm"
        };

        public static string NormalizeRelPath(string rawPath)
        {
            if (string.IsNullOrWhiteSpace(rawPath))
                return null;
            string p = rawPath.Replace('\\', '/').Trim().Trim('/');
            if (string.IsNullOrEmpty(p) || p.Contains(".."))
                return null;
            return p;
        }

        public static bool IsIgnored(string normPath)
        {
            if (string.IsNullOrEmpty(normPath))
                return true;
            string lower = normPath.ToLowerInvariant();
            int slash = lower.IndexOf('/');
            string firstSeg = slash >= 0 ? lower.Substring(0, slash) : lower;

            foreach (var r in IgnoredRoots)
            {
                if (firstSeg == r)
                    return true;
            }

            if (lower.StartsWith("packages/com.vrchat.") || lower.StartsWith("assets/serializedudonprograms/"))
                return true;

            if (lower.EndsWith(".csproj") || lower.EndsWith(".sln") || lower.EndsWith(".suo") ||
                lower.EndsWith(".user") || lower.EndsWith(".userprefs") || lower.EndsWith(".pidb") ||
                lower.EndsWith(".booproj") || lower.EndsWith(".svd") || lower.EndsWith(".pdb") ||
                lower.EndsWith(".mdb") || lower.EndsWith(".opendb") || lower.EndsWith(".vc.db") ||
                lower.EndsWith(".ds_store") || lower == "thumbs.db")
            {
                return true;
            }

            return false;
        }

        public static bool IsLockableAsset(string normPath)
        {
            if (string.IsNullOrEmpty(normPath))
                return false;
            string lower = normPath.ToLowerInvariant();
            foreach (var ext in LockableExtensions)
            {
                if (lower.EndsWith(ext))
                    return true;
            }
            return false;
        }

        public static string ComputeSha256Hex(byte[] bytes)
        {
            using var sha = SHA256.Create();
            byte[] hash = sha.ComputeHash(bytes);
            char[] c = new char[hash.Length * 2];
            const string hex = "0123456789abcdef";
            for (int i = 0; i < hash.Length; i++)
            {
                c[i * 2] = hex[hash[i] >> 4];
                c[i * 2 + 1] = hex[hash[i] & 0x0F];
            }
            return new string(c);
        }
    }
}
