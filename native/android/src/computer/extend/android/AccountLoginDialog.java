package computer.extend.android;

import android.app.Activity;
import android.app.AlertDialog;
import android.text.InputType;
import android.view.WindowManager;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.TextView;
import org.json.JSONObject;

/** API-only fallback. Passwords, MFA tickets and codes never cross RN. */
public final class AccountLoginDialog {
    private AccountLoginDialog() {}
    public static void show(Activity activity,Accounts accounts) {
        activity.runOnUiThread(()->{
            Form form=null;try{form=new Form(activity,accounts);form.open();}catch(Exception failed){accounts.nativeFailed(form==null?-1:form.generation);}
        });
    }
    private static final class Form {
        final Activity activity;
        final Accounts accounts;
        final EditText email,password,code;
        final TextView message;
        final AlertDialog dialog;
        long generation;
        String ticket;
        boolean busy;
        Form(Activity activity,Accounts accounts){
            this.activity=activity;this.accounts=accounts;
            LinearLayout fields=new LinearLayout(activity);fields.setOrientation(LinearLayout.VERTICAL);
            int inset=(int)(24*activity.getResources().getDisplayMetrics().density);fields.setPadding(inset,inset,inset,inset);
            email=new EditText(activity);email.setHint("Email");email.setContentDescription("Account email");email.setInputType(InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS);fields.addView(email);
            password=new EditText(activity);password.setHint("Password");password.setContentDescription("Account password");password.setInputType(InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_VARIATION_PASSWORD);fields.addView(password);
            code=new EditText(activity);code.setHint("Authenticator or recovery code");code.setContentDescription("Authenticator or recovery code");code.setInputType(InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD);code.setVisibility(android.view.View.GONE);fields.addView(code);
            message=new TextView(activity);message.setText("Sign in on this device. Use browser sign-in for passkeys or security keys.");message.setAccessibilityLiveRegion(android.view.View.ACCESSIBILITY_LIVE_REGION_POLITE);fields.addView(message);
            dialog=new AlertDialog.Builder(activity).setTitle("Sign in to your account").setView(fields).setNegativeButton("Cancel",(d,w)->{}).setPositiveButton("Sign in",null).create();
        }
        void open()throws Exception{
            generation=accounts.beginNative(()->activity.runOnUiThread(dialog::dismiss));
            dialog.setOnDismissListener(d->{email.getText().clear();password.getText().clear();code.getText().clear();ticket=null;accounts.nativeClosed(generation);});
            dialog.show();dialog.getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);
            dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener(v->submit());
        }
        void submit(){
            if(busy)return;
            String enteredEmail=email.getText().toString().trim(),enteredPassword=password.getText().toString(),enteredCode=code.getText().toString().trim(),pending=ticket;
            if(pending==null && (enteredEmail.isEmpty() || enteredPassword.isEmpty())){message.setText("Enter your email and password.");return;}
            if(pending!=null && enteredCode.isEmpty()){message.setText("Enter your authenticator or recovery code.");return;}
            password.getText().clear();code.getText().clear();busy=true;dialog.getButton(AlertDialog.BUTTON_POSITIVE).setEnabled(false);
            accounts.submitNative(()->{
                JSONObject result=null;String failure=null;
                try{result=pending==null?accounts.password(generation,enteredEmail,enteredPassword):accounts.verify(generation,pending,enteredCode);}
                catch(Exception failed){failure=failed instanceof AccountHttp.Failure?failed.getMessage():"Couldn't sign in. Check your account details and try again.";}
                JSONObject response=result;String error=failure;
                activity.runOnUiThread(()->{
                    if(!dialog.isShowing())return;busy=false;dialog.getButton(AlertDialog.BUTTON_POSITIVE).setEnabled(true);
                    if(error!=null){message.setText(error);return;}
                    if(response.optBoolean("mfa_required",false)){
                        ticket=response.optString("ticket");email.setVisibility(android.view.View.GONE);password.setVisibility(android.view.View.GONE);code.setVisibility(android.view.View.VISIBLE);code.requestFocus();
                        message.setText(response.optBoolean("totp",false)||response.optBoolean("recovery",false)?"Enter an authenticator or recovery code.":"This account requires a security key. Cancel and use browser sign-in.");
                        dialog.getButton(AlertDialog.BUTTON_POSITIVE).setText("Verify");
                    }else{dialog.dismiss();}
                });
            });
        }
    }
}
