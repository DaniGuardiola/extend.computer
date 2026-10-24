package computer.extend.android.tests;
import android.app.Activity;
import android.os.Bundle;
import android.text.Editable;
import android.text.TextWatcher;
import android.view.MotionEvent;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

/** Disposable native input target; never ships in the application. */
public final class ReceiverTargetActivity extends Activity {
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);getSharedPreferences("input-target",0).edit().clear().commit();
        LinearLayout content=new LinearLayout(this);content.setOrientation(LinearLayout.VERTICAL);content.setPadding(40,80,40,40);
        TextView title=new TextView(this);title.setText("Extend receiver input check");title.setTextSize(28);content.addView(title);
        TextView touch=new TextView(this);touch.setText("Touch idle");content.addView(touch);
        Button button=new Button(this) {
            int moves;
            @Override public boolean onTouchEvent(MotionEvent event) {
                if(event.getActionMasked()==MotionEvent.ACTION_DOWN){touch.setText("Touch held");getSharedPreferences("input-target",0).edit().putString("touch","held").commit();}
                if(event.getActionMasked()==MotionEvent.ACTION_UP || event.getActionMasked()==MotionEvent.ACTION_CANCEL){touch.setText("Touch released");getSharedPreferences("input-target",0).edit().putString("touch","released").commit();}
                if(event.getActionMasked()==MotionEvent.ACTION_MOVE)getSharedPreferences("input-target",0).edit().putInt("moves",++moves).commit();
                return super.onTouchEvent(event);
            }
        };button.setText("Remote click target");button.setContentDescription("Remote click target");
        button.setOnClickListener(v->button.setText("Remote click received"));content.addView(button,new LinearLayout.LayoutParams(-1,140));
        EditText field=new EditText(this);field.setSingleLine(true);field.setHint("Remote typing target");field.setContentDescription("Remote typing target");
        field.addTextChangedListener(new TextWatcher() {
            int changes;
            @Override public void beforeTextChanged(CharSequence value,int start,int count,int after) {}
            @Override public void onTextChanged(CharSequence value,int start,int before,int count) {}
            @Override public void afterTextChanged(Editable value) {
                // Only this disposable test editor is observed. No other
                // application or accessibility text is retained.
                getSharedPreferences("input-target",0).edit().putString("text",value.toString())
                    .putInt("text-utf16",value.length()).putInt("text-changes",++changes).commit();
            }
        });
        content.addView(field,new LinearLayout.LayoutParams(-1,140));
        ScrollView scroll=new ScrollView(this) {
            @Override protected void onScrollChanged(int x,int y,int oldX,int oldY) { super.onScrollChanged(x,y,oldX,oldY);getSharedPreferences("input-target",0).edit().putInt("scroll",y).commit(); }
        };scroll.setContentDescription("Remote scroll target");LinearLayout rows=new LinearLayout(this);rows.setOrientation(LinearLayout.VERTICAL);
        for(int i=0;i<40;i++){TextView row=new TextView(this);row.setText("Scroll row "+i);row.setTextSize(24);row.setPadding(20,40,20,40);rows.addView(row);}
        scroll.addView(rows);content.addView(scroll,new LinearLayout.LayoutParams(-1,0,1));setContentView(content);
    }
}
