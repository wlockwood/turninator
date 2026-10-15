# Native cache directory

`EmbeddedConfig::cache_directory: Option<PathBuf>` transports an application-owned
cache directory to native renderers. The trusted application adapter creates and
owns the root before passing it to `EmbeddedLibrary::create`. The loader validates
the directory; it cannot prove ownership from a public method argument alone.

When supplied, the host requires an absolute Windows drive or UNC path to an
existing directory. It resolves the directory with `canonicalize`, then sends
the resolved UTF-8 string, bounded to 4096 bytes with no NUL. Empty, relative,
rooted-only, device namespace, missing, file, invalid UTF-8 and overlong paths
fail before the native create call. The loader does not create directories or
change environment variables.

The schema1 config JSON has an optional top-level `cache_directory` string.
`None` omits the field entirely, preserving the legacy config payload. No C ABI
struct layout or export changes are needed. The strict native schema accepts
drive, UNC, verbatim drive and verbatim UNC paths (including Unicode), rejects
invalid types and path syntax, and copies the string without opening paths or
mutating the filesystem or environment. All other schema guards still apply.

The production LuxCore backend must require and use this explicit private root;
it must not fall back to a global cache. Its bridge supplies verified runtime
compiler and SDK identities to the private SDK cache properties. Callers cannot
set those identities through this cache-directory option. Filament may ignore
an absent directory. These requirements describe the backend integration
boundary; the CPU fixture only verifies the transport and is not a production
renderer.

The directory is a storage location, not part of `BackendIdentity` or the pixel
recipe. Moving an otherwise identical cache must not change pixel identity or
rendered output. Actual compiler, SDK and recipe identities continue to determine
cache validity independently of its location.
