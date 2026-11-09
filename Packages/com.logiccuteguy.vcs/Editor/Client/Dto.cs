using System;
using System.Collections.Generic;
using UnityEngine;

namespace LogicCuteGuy.Vcs.Editor.Client
{
    [Serializable]
    public class LoginRequest
    {
        public string username;
        public string password;
    }

    [Serializable]
    public class LoginResponse
    {
        public string token;
        public string username;
        public bool is_admin;
    }

    [Serializable]
    public class RepoInfo
    {
        public string name;
        public string description;
        public long head_changeset_id;
        public long created_at_unix;
    }

    [Serializable]
    public class CreateRepoRequest
    {
        public string name;
        public string description;
    }

    [Serializable]
    public class BranchInfo
    {
        public string name;
        public long head_changeset_id;
        public string created_by;
        public long created_at_unix;
    }

    [Serializable]
    public class CreateBranchRequest
    {
        public string name;
        public long from_changeset_id;
    }

    [Serializable]
    public class FileEntry
    {
        public string path;
        public string sha256;
        public ulong size;
    }

    [Serializable]
    public class FileChangeItem
    {
        public string path;
        public string kind; // "added" | "modified" | "deleted"
        public string sha256;
        public ulong size;
    }

    [Serializable]
    public class CheckinRequest
    {
        public string branch;
        public long parent_changeset_id;
        public string comment;
        public List<FileChangeItem> changes = new List<FileChangeItem>();
    }

    [Serializable]
    public class CheckinResponse
    {
        public long changeset_id;
        public string branch;
        public bool rebased;
    }

    [Serializable]
    public class ChangesetSummary
    {
        public long id;
        public string branch;
        public long parent_id;
        public string author;
        public string comment;
        public long timestamp_unix;
        public int change_count;
    }

    [Serializable]
    public class ChangesetDetail
    {
        public ChangesetSummary summary;
        public List<FileChangeItem> changes = new List<FileChangeItem>();
    }

    [Serializable]
    public class TreeResponse
    {
        public long changeset_id;
        public string branch;
        public List<FileEntry> files = new List<FileEntry>();
    }

    [Serializable]
    public class MissingBlobsRequest
    {
        public List<string> hashes = new List<string>();
    }

    [Serializable]
    public class MissingBlobsResponse
    {
        public List<string> missing = new List<string>();
    }

    [Serializable]
    public class LockInfo
    {
        public string path;
        public string branch;
        public string owner;
        public long locked_at_unix;
    }

    [Serializable]
    public class AcquireLocksRequest
    {
        public string branch;
        public List<string> paths = new List<string>();
    }

    [Serializable]
    public class ReleaseLocksRequest
    {
        public string branch;
        public List<string> paths = new List<string>();
        public bool force;
    }

    [Serializable]
    public class ErrorResponse
    {
        public string error;
    }

    public static class JsonArrayHelper
    {
        [Serializable]
        private class Wrapper<T>
        {
            public List<T> items = new List<T>();
        }

        public static List<T> FromJsonArray<T>(string json)
        {
            if (string.IsNullOrWhiteSpace(json))
                return new List<T>();
            string wrapped = "{\"items\":" + json.Trim() + "}";
            var w = JsonUtility.FromJson<Wrapper<T>>(wrapped);
            return w != null && w.items != null ? w.items : new List<T>();
        }
    }
}
