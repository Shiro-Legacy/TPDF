package com.tandempdf.mobile;

import android.app.Activity;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.util.Base64;
import androidx.activity.result.ActivityResultLauncher;
import androidx.activity.result.IntentSenderRequest;
import androidx.activity.result.contract.ActivityResultContracts;
import com.getcapacitor.JSArray;
import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;
import com.google.mlkit.vision.common.InputImage;
import com.google.mlkit.vision.documentscanner.GmsDocumentScannerOptions;
import com.google.mlkit.vision.documentscanner.GmsDocumentScanning;
import com.google.mlkit.vision.documentscanner.GmsDocumentScanningResult;
import com.google.mlkit.vision.text.TextRecognition;
import com.google.mlkit.vision.text.TextRecognizer;
import com.google.mlkit.vision.text.latin.TextRecognizerOptions;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

@CapacitorPlugin(name = "DocumentScanner")
public class DocumentScannerPlugin extends Plugin {
    private final ExecutorService executor = Executors.newSingleThreadExecutor();
    private ActivityResultLauncher<IntentSenderRequest> scannerLauncher;
    private PluginCall pendingScan;

    @PluginMethod
    public void scan(PluginCall call) {
        int limit = call.getInt("pageLimit", 12);
        if (limit < 1 || limit > 12) { call.reject("A scan can contain up to 12 pages."); return; }
        getActivity().runOnUiThread(() -> {
            if (pendingScan != null) { call.reject("A scan is already open."); return; }
            pendingScan = call;
            if (scannerLauncher == null) {
                scannerLauncher = getActivity().getActivityResultRegistry().register("tandem-document-scan",
                    new ActivityResultContracts.StartIntentSenderForResult(), result -> {
                        PluginCall pending = pendingScan;
                        pendingScan = null;
                        if (pending == null) return;
                        if (result.getResultCode() != Activity.RESULT_OK) { pending.resolve(new JSObject().put("images", new JSArray())); return; }
                        GmsDocumentScanningResult scan = GmsDocumentScanningResult.fromActivityResultIntent(result.getData());
                        if (scan == null || scan.getPages() == null || scan.getPages().size() > 12) { pending.reject("The scan could not be read."); return; }
                        executor.execute(() -> {
                            JSArray images = new JSArray();
                            int total = 0;
                            try {
                                for (GmsDocumentScanningResult.Page page : scan.getPages()) {
                                    // Inspect before decoding to avoid allocating a full camera-resolution bitmap.
                                    BitmapFactory.Options options = new BitmapFactory.Options(); options.inJustDecodeBounds = true;
                                    try (InputStream in = getContext().getContentResolver().openInputStream(page.getImageUri())) { BitmapFactory.decodeStream(in, null, options); }
                                    options.inSampleSize = 1;
                                    while (Math.max(options.outWidth, options.outHeight) / options.inSampleSize > 4800) options.inSampleSize *= 2;
                                    options.inJustDecodeBounds = false;
                                    Bitmap bitmap;
                                    try (InputStream in = getContext().getContentResolver().openInputStream(page.getImageUri())) { bitmap = BitmapFactory.decodeStream(in, null, options); }
                                    if (bitmap == null) throw new IllegalArgumentException();
                                    float scale = Math.min(1f, 2400f / Math.max(bitmap.getWidth(), bitmap.getHeight()));
                                    Bitmap resized = Bitmap.createScaledBitmap(bitmap, Math.max(1, Math.round(bitmap.getWidth() * scale)), Math.max(1, Math.round(bitmap.getHeight() * scale)), true);
                                    ByteArrayOutputStream out = new ByteArrayOutputStream();
                                    resized.compress(Bitmap.CompressFormat.JPEG, 88, out);
                                    if (resized != bitmap) resized.recycle(); bitmap.recycle();
                                    byte[] data = out.toByteArray(); total += data.length;
                                    if (total > 30 * 1024 * 1024) { pending.reject("The scans exceed 30 MB. Scan fewer pages."); return; }
                                    images.put(Base64.encodeToString(data, Base64.NO_WRAP));
                                }
                                pending.resolve(new JSObject().put("images", images));
                            } catch (Exception error) { pending.reject("The scan could not be read. Try Add photos."); }
                        });
                    });
            }
            GmsDocumentScannerOptions options = new GmsDocumentScannerOptions.Builder()
                .setGalleryImportAllowed(true).setPageLimit(limit)
                .setResultFormats(GmsDocumentScannerOptions.RESULT_FORMAT_JPEG)
                .setScannerMode(GmsDocumentScannerOptions.SCANNER_MODE_FULL).build();
            GmsDocumentScanning.getClient(options).getStartScanIntent(getActivity())
                .addOnSuccessListener(sender -> scannerLauncher.launch(new IntentSenderRequest.Builder(sender).build()))
                .addOnFailureListener(error -> { pendingScan = null; call.reject("The scanner requires Google Play services and may need an initial download. Use Add photos if unavailable."); });
        });
    }

    @PluginMethod
    public void recognize(PluginCall call) {
        String encoded = call.getString("data");
        if (encoded == null || encoded.length() > 16_000_000) { call.reject("Invalid scan image."); return; }
        executor.execute(() -> {
            try {
                byte[] bytes = Base64.decode(encoded, Base64.DEFAULT);
                BitmapFactory.Options options = new BitmapFactory.Options(); options.inJustDecodeBounds = true;
                BitmapFactory.decodeByteArray(bytes, 0, bytes.length, options);
                if (options.outWidth < 1 || options.outHeight < 1 || options.outWidth > 2400 || options.outHeight > 2400) { call.reject("Invalid scan dimensions."); return; }
                Bitmap bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.length);
                if (bitmap == null) { call.reject("Invalid scan image."); return; }
                TextRecognizer recognizer = TextRecognition.getClient(TextRecognizerOptions.DEFAULT_OPTIONS);
                recognizer.process(InputImage.fromBitmap(bitmap, 0)).addOnSuccessListener(result -> {
                    JSArray lines = new JSArray();
                    for (com.google.mlkit.vision.text.Text.TextBlock block : result.getTextBlocks()) {
                        for (com.google.mlkit.vision.text.Text.Line line : block.getLines()) {
                            android.graphics.Rect rect = line.getBoundingBox();
                            if (rect == null || rect.width() <= 0 || rect.height() <= 0) continue;
                            double x = Math.max(0, (double) rect.left / bitmap.getWidth()), y = Math.max(0, (double) rect.top / bitmap.getHeight());
                            lines.put(new JSObject().put("text", line.getText()).put("x", x).put("y", y)
                                .put("width", Math.min(1 - x, (double) rect.width() / bitmap.getWidth()))
                                .put("height", Math.min(1 - y, (double) rect.height() / bitmap.getHeight())));
                        }
                    }
                    call.resolve(new JSObject().put("lines", lines));
                }).addOnFailureListener(error -> call.reject("Text recognition failed. Try a sharper photo."))
                  .addOnCompleteListener(result -> { recognizer.close(); bitmap.recycle(); });
            } catch (Exception error) { call.reject("Text recognition failed. Try another image."); }
        });
    }
    @Override protected void handleOnDestroy() {
        if (scannerLauncher != null) scannerLauncher.unregister();
        executor.shutdown();
        if (pendingScan != null) pendingScan.reject("The scanner was closed. Please try again.");
        pendingScan = null;
    }
}
