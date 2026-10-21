package org.rustymcla.game;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import java.io.File;
import java.io.FileNotFoundException;

/** Grants a chosen sharing app read access to one diagnostic report only. */
public final class DiagnosticProvider extends ContentProvider {
    private File file(Uri uri) {
        if (!"/report.txt".equals(uri.getPath())) throw new IllegalArgumentException("Unknown report");
        return new File(getContext().getFilesDir(), "diagnostic-report.txt");
    }
    @Override public boolean onCreate() { return true; }
    @Override public String getType(Uri uri) { file(uri); return "text/plain"; }
    @Override public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (!"r".equals(mode)) throw new FileNotFoundException("Read only");
        return ParcelFileDescriptor.open(file(uri), ParcelFileDescriptor.MODE_READ_ONLY);
    }
    @Override public Cursor query(Uri uri, String[] projection, String selection, String[] args, String order) {
        File report = file(uri);
        String[] columns = projection == null ? new String[] {OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE} : projection;
        MatrixCursor cursor = new MatrixCursor(columns);
        Object[] row = new Object[columns.length];
        for (int i = 0; i < columns.length; i++) {
            if (OpenableColumns.DISPLAY_NAME.equals(columns[i])) row[i] = "Rusty-MCLA-diagnostico.txt";
            if (OpenableColumns.SIZE.equals(columns[i])) row[i] = report.length();
        }
        cursor.addRow(row);
        return cursor;
    }
    @Override public Uri insert(Uri uri, ContentValues values) { throw new UnsupportedOperationException(); }
    @Override public int update(Uri uri, ContentValues values, String where, String[] args) { throw new UnsupportedOperationException(); }
    @Override public int delete(Uri uri, String where, String[] args) { throw new UnsupportedOperationException(); }
}
