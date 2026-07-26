package com.emuchef.fixture;

import android.app.Activity;
import android.os.Bundle;
import android.view.Gravity;
import android.widget.TextView;

/**
 * Deliberately small launcher activity used only for non-root ADB qualification.
 *
 * <p>The activity does not access the network, accounts, analytics SDKs, or
 * device hardware. Its manifest declares CAMERA solely so the executor can
 * qualify permission and app-op handling against a disposable package.</p>
 */
public final class MainActivity extends Activity {
    @Override
    public void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        TextView message = new TextView(this);
        message.setGravity(Gravity.CENTER);
        message.setText("EmuChef non-root qualification fixture");
        setContentView(message);
    }
}
