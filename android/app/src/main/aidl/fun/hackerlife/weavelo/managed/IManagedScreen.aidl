package fun.hackerlife.weavelo.managed;
import android.view.Surface;
import android.view.MotionEvent;
import android.view.KeyEvent;
/** Local UID-bound display operations; never a network or command endpoint. */
interface IManagedScreen {
    String info();
    int createMirror(int width, int height, int density);
    String applications();
    byte[] applicationIcon(String packageName, int edge);
    int createApplication(String packageName, int width, int height, int density);
    boolean injectMotion(int id, in MotionEvent event);
    boolean injectKey(int id, in KeyEvent event);
    void setSurface(int id, in Surface surface);
    void resize(int id, int width, int height, int density);
    void release(int id);
}
