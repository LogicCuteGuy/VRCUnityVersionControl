using System;
using System.Collections.Generic;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text;
using System.Threading.Tasks;
using UnityEngine;

namespace LogicCuteGuy.Vcs.Editor.Client
{
    public class ApiException : Exception
    {
        public int StatusCode { get; }

        public ApiException(int statusCode, string message) : base(message)
        {
            StatusCode = statusCode;
        }
    }

    public static class ApiClient
    {
        private static readonly HttpClient Http = new HttpClient
        {
            Timeout = TimeSpan.FromMinutes(5)
        };

        private static string CleanServer(string url) => (url ?? "").Trim().TrimEnd('/');

        private static HttpRequestMessage CreateRequest(HttpMethod method, string url, string token, HttpContent content = null)
        {
            var req = new HttpRequestMessage(method, url);
            if (!string.IsNullOrEmpty(token))
            {
                req.Headers.Authorization = new AuthenticationHeaderValue("Bearer", token.Trim());
            }
            if (content != null)
            {
                req.Content = content;
            }
            return req;
        }

        private static async Task<string> SendJsonAsync(HttpMethod method, string url, string token, object payload = null)
        {
            HttpContent content = null;
            if (payload != null)
            {
                string jsonBody = JsonUtility.ToJson(payload);
                content = new StringContent(jsonBody, Encoding.UTF8, "application/json");
            }

            using var req = CreateRequest(method, url, token, content);
            using var resp = await Http.SendAsync(req).ConfigureAwait(false);
            string text = await resp.Content.ReadAsStringAsync().ConfigureAwait(false);

            if (!resp.IsSuccessStatusCode)
            {
                string errMsg = text;
                try
                {
                    var errObj = JsonUtility.FromJson<ErrorResponse>(text);
                    if (errObj != null && !string.IsNullOrEmpty(errObj.error))
                        errMsg = errObj.error;
                }
                catch { }
                throw new ApiException((int)resp.StatusCode, errMsg);
            }

            return text;
        }

        public static async Task<bool> CheckHealthAsync(string serverUrl)
        {
            try
            {
                using var req = new HttpRequestMessage(HttpMethod.Get, $"{CleanServer(serverUrl)}/api/v1/health");
                using var resp = await Http.SendAsync(req).ConfigureAwait(false);
                return resp.IsSuccessStatusCode;
            }
            catch
            {
                return false;
            }
        }

        public static async Task<LoginResponse> LoginAsync(string serverUrl, string username, string password)
        {
            string text = await SendJsonAsync(
                HttpMethod.Post,
                $"{CleanServer(serverUrl)}/api/v1/login",
                null,
                new LoginRequest { username = username, password = password }
            ).ConfigureAwait(false);
            return JsonUtility.FromJson<LoginResponse>(text);
        }

        public static async Task<List<RepoInfo>> ListReposAsync(string serverUrl, string token)
        {
            string text = await SendJsonAsync(
                HttpMethod.Get,
                $"{CleanServer(serverUrl)}/api/v1/repos",
                token
            ).ConfigureAwait(false);
            return JsonArrayHelper.FromJsonArray<RepoInfo>(text);
        }

        public static async Task<RepoInfo> CreateRepoAsync(string serverUrl, string token, string name, string description)
        {
            string text = await SendJsonAsync(
                HttpMethod.Post,
                $"{CleanServer(serverUrl)}/api/v1/repos",
                token,
                new CreateRepoRequest { name = name, description = description }
            ).ConfigureAwait(false);
            return JsonUtility.FromJson<RepoInfo>(text);
        }

        public static async Task<List<BranchInfo>> ListBranchesAsync(string serverUrl, string token, string repo)
        {
            string text = await SendJsonAsync(
                HttpMethod.Get,
                $"{CleanServer(serverUrl)}/api/v1/repos/{Uri.EscapeDataString(repo)}/branches",
                token
            ).ConfigureAwait(false);
            return JsonArrayHelper.FromJsonArray<BranchInfo>(text);
        }

        public static async Task<BranchInfo> CreateBranchAsync(string serverUrl, string token, string repo, string branchName, long fromChangesetId)
        {
            string text = await SendJsonAsync(
                HttpMethod.Post,
                $"{CleanServer(serverUrl)}/api/v1/repos/{Uri.EscapeDataString(repo)}/branches",
                token,
                new CreateBranchRequest { name = branchName, from_changeset_id = fromChangesetId }
            ).ConfigureAwait(false);
            return JsonUtility.FromJson<BranchInfo>(text);
        }

        public static async Task<List<ChangesetSummary>> ListChangesetsAsync(string serverUrl, string token, string repo, string branch = null, int limit = 50)
        {
            string query = $"?limit={limit}";
            if (!string.IsNullOrEmpty(branch))
                query += $"&branch={Uri.EscapeDataString(branch)}";
            string text = await SendJsonAsync(
                HttpMethod.Get,
                $"{CleanServer(serverUrl)}/api/v1/repos/{Uri.EscapeDataString(repo)}/changesets{query}",
                token
            ).ConfigureAwait(false);
            return JsonArrayHelper.FromJsonArray<ChangesetSummary>(text);
        }

        public static async Task<TreeResponse> GetLatestTreeAsync(string serverUrl, string token, string repo, string branch)
        {
            string text = await SendJsonAsync(
                HttpMethod.Get,
                $"{CleanServer(serverUrl)}/api/v1/repos/{Uri.EscapeDataString(repo)}/changesets/latest/tree?branch={Uri.EscapeDataString(branch)}",
                token
            ).ConfigureAwait(false);
            return JsonUtility.FromJson<TreeResponse>(text);
        }

        public static async Task<List<string>> GetMissingBlobsAsync(string serverUrl, string token, string repo, List<string> hashes)
        {
            string text = await SendJsonAsync(
                HttpMethod.Post,
                $"{CleanServer(serverUrl)}/api/v1/repos/{Uri.EscapeDataString(repo)}/blobs/missing",
                token,
                new MissingBlobsRequest { hashes = hashes }
            ).ConfigureAwait(false);
            var resp = JsonUtility.FromJson<MissingBlobsResponse>(text);
            return resp != null && resp.missing != null ? resp.missing : new List<string>();
        }

        public static async Task UploadBlobAsync(string serverUrl, string token, string repo, string sha256, byte[] bytes)
        {
            using var content = new ByteArrayContent(bytes);
            content.Headers.ContentType = new MediaTypeHeaderValue("application/octet-stream");
            using var req = CreateRequest(
                HttpMethod.Put,
                $"{CleanServer(serverUrl)}/api/v1/repos/{Uri.EscapeDataString(repo)}/blobs/{sha256}",
                token,
                content
            );
            using var resp = await Http.SendAsync(req).ConfigureAwait(false);
            if (!resp.IsSuccessStatusCode)
            {
                string err = await resp.Content.ReadAsStringAsync().ConfigureAwait(false);
                throw new ApiException((int)resp.StatusCode, $"Upload blob {sha256} failed: {err}");
            }
        }

        public static async Task<byte[]> DownloadBlobAsync(string serverUrl, string token, string repo, string sha256)
        {
            using var req = CreateRequest(
                HttpMethod.Get,
                $"{CleanServer(serverUrl)}/api/v1/repos/{Uri.EscapeDataString(repo)}/blobs/{sha256}",
                token
            );
            using var resp = await Http.SendAsync(req).ConfigureAwait(false);
            if (!resp.IsSuccessStatusCode)
            {
                string err = await resp.Content.ReadAsStringAsync().ConfigureAwait(false);
                throw new ApiException((int)resp.StatusCode, $"Download blob {sha256} failed: {err}");
            }
            return await resp.Content.ReadAsByteArrayAsync().ConfigureAwait(false);
        }

        public static async Task<CheckinResponse> CheckinAsync(string serverUrl, string token, string repo, CheckinRequest request)
        {
            string text = await SendJsonAsync(
                HttpMethod.Post,
                $"{CleanServer(serverUrl)}/api/v1/repos/{Uri.EscapeDataString(repo)}/checkin",
                token,
                request
            ).ConfigureAwait(false);
            return JsonUtility.FromJson<CheckinResponse>(text);
        }

        public static async Task<List<LockInfo>> ListLocksAsync(string serverUrl, string token, string repo, string branch)
        {
            string text = await SendJsonAsync(
                HttpMethod.Get,
                $"{CleanServer(serverUrl)}/api/v1/repos/{Uri.EscapeDataString(repo)}/locks?branch={Uri.EscapeDataString(branch)}",
                token
            ).ConfigureAwait(false);
            return JsonArrayHelper.FromJsonArray<LockInfo>(text);
        }

        public static async Task<List<LockInfo>> AcquireLocksAsync(string serverUrl, string token, string repo, string branch, List<string> paths)
        {
            string text = await SendJsonAsync(
                HttpMethod.Post,
                $"{CleanServer(serverUrl)}/api/v1/repos/{Uri.EscapeDataString(repo)}/locks",
                token,
                new AcquireLocksRequest { branch = branch, paths = paths }
            ).ConfigureAwait(false);
            return JsonArrayHelper.FromJsonArray<LockInfo>(text);
        }

        public static async Task<List<string>> ReleaseLocksAsync(string serverUrl, string token, string repo, string branch, List<string> paths, bool force)
        {
            string text = await SendJsonAsync(
                HttpMethod.Post,
                $"{CleanServer(serverUrl)}/api/v1/repos/{Uri.EscapeDataString(repo)}/locks/unlock",
                token,
                new ReleaseLocksRequest { branch = branch, paths = paths, force = force }
            ).ConfigureAwait(false);
            return JsonArrayHelper.FromJsonArray<string>(text);
        }
    }
}
