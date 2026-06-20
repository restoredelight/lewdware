# pack-editor

This is a GUI tool for creating and editing packs, written using Tauri and Svelte.
Run it using `pnpm tauri dev`.

## The pack format

Lewdware packs (.lwpack) are a custom file format. A pack file consists of the
following (in order):

- A fixed-size header.
- Sequentially-stored image, video and audio files.
- An SQLite database.
- Other metadata encoded as binary.

The header stores the offset and sizes of the database and metadata, allowing
them to be read. It also stores a UUID that is assigned to the file when it
is created.

The metadata stores some very basic metadata attached to the pack (name, author,
version, etc.).

The SQLite database stores the name, offset and size of each media file, along with
some other data (e.g. tags, a thumbnail, a hash). It also stores the modes attached
to the pack.

## Reading/Saving

The pack editor does its best to avoid corrupting the file, and only ever actually
writes to the file when the user clicks the save button.

When a pack is first opened and read, the pack editor copies the database file
and the metadata (encoded as a binary file) to a custom data directory (based
on the pack's UUID), and opens an SQLite connection to that copied file.

When a user makes a change to the pack, we only change files in this directory.
If the user uploads a media file, the pack editor stores the compressed file
in this directory, and adds a record to the database pointing to it. If the
user deletes or modifies a media file, we only update the database. Similarly,
if the metadata is updated, we update the metadata file in this directory.
Finally, after any change, we create a file named "UNSAVED" in this directory
to signify that we have unsaved changes.

Upon saving, the following steps are taken:

- We fetch all media files from the database, ordered by their offset, with
  newly added files at the end.
- We write these files to the pack file, in order (after the header). The
  invariant here is that, for an existing file, its new offset will be less
  than or equal to its old offset - ensuring that we never overwrite files that
  we have not already written. We skip files whose offset have not changed.
- After each file is written, we update its database entry to point to its
  new offset, ensuring that the database is correct after each written file.
- The pack editor writes the database and metadata to the end of the file, and
  update the header to point to them.
- Finally, the "UNSAVED" file is deleted.

When the user tries to close the pack editor and they haven't saved, we
prompt them to save or discard their changes. However, if a pack is closed
without saving (e.g. due to a crash), and then re-opened, all the changes are
saved in the directory, and so the pack editor sees the existing "UNSAVED" file
and prompts the user to either load or discard the changes. If the user chooses
to load the changes (the default), then we use the existing database and
metadata files in the directory.

Since we can rely on SQLite to be durable and reliable in the event of a crash
or power failure, we can guarantee that the only time any data can be lost is
if the program is closed while in the middle of a save operation. Even in this
case, only a single media file can become corrupted.

Another approach to preventing data corruption that we could have taken is to
write to a copy of the pack file, and then move it to the original location.
However, we expect pack files to be quite big, and so we avoid doing this to
save storage space.

## Encoding files

Images are encoded as AVIF files and audio files/streams are encoded using
Opus, to maximise compression savings. Videos, however, are encoded using
H.264 for fast decoding (see the `lewdware` README for details).

We detect when images and videos are transparent (/not opaque), and store
this information in the database. We use a special encoding method for
transparent videos (again, see the `lewdware` README for why we do this).
