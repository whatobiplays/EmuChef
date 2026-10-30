package org.emuchef.t03.copyfixture;

import android.app.Activity;
import android.os.Bundle;

import java.io.File;

/** Creates disposable private directories used by generic App Target tests. */
public final class FixtureActivity extends Activity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        File files = getFilesDir();
        requireDirectory(new File(files, "assets"));
        requireDirectory(new File(files, "sibling"));
        finish();
    }

    private static void requireDirectory(File directory) {
        if (!directory.isDirectory() && !directory.mkdirs()) {
            throw new IllegalStateException("Unable to create test directory: " + directory);
        }
    }
}
