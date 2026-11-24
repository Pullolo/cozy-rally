// Generates the launcher icon (res/mipmap-*/ic_launcher.png) in code, like every other asset.
// A cozy pastel badge: sky gradient, rolling green hill, winding road and a little red rally car.
// Usage (in the build container): java -Djava.awt.headless=true MakeIcon.java <res dir>
import java.awt.*;
import java.awt.geom.*;
import java.awt.image.BufferedImage;
import java.io.File;
import javax.imageio.ImageIO;

public class MakeIcon {
    public static void main(String[] args) throws Exception {
        String[][] sizes = {{"mdpi", "48"}, {"hdpi", "72"}, {"xhdpi", "96"}, {"xxhdpi", "144"}, {"xxxhdpi", "192"}};
        for (String[] s : sizes) {
            int n = Integer.parseInt(s[1]);
            File dir = new File(args[0], "mipmap-" + s[0]);
            dir.mkdirs();
            ImageIO.write(draw(n), "png", new File(dir, "ic_launcher.png"));
        }
    }

    static BufferedImage draw(int n) {
        BufferedImage img = new BufferedImage(n, n, BufferedImage.TYPE_INT_ARGB);
        Graphics2D g = img.createGraphics();
        g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
        g.setRenderingHint(RenderingHints.KEY_RENDERING, RenderingHints.VALUE_RENDER_QUALITY);
        g.scale(n / 100.0, n / 100.0);
        Shape badge = new RoundRectangle2D.Double(2, 2, 96, 96, 34, 34);
        g.setClip(badge);
        // sky
        g.setPaint(new GradientPaint(0, 0, new Color(0xFFD9A8), 0, 60, new Color(0xA8D8F0)));
        g.fill(badge);
        // sun
        g.setColor(new Color(0xFFF3C4));
        g.fill(new Ellipse2D.Double(60, 14, 22, 22));
        // far hill
        g.setColor(new Color(0x9BCF8E));
        g.fill(new Ellipse2D.Double(-30, 48, 110, 70));
        // near hill
        g.setColor(new Color(0x7CBF6E));
        g.fill(new Ellipse2D.Double(20, 56, 120, 70));
        // winding road
        Path2D road = new Path2D.Double();
        road.moveTo(30, 102);
        road.curveTo(40, 84, 78, 82, 62, 66);
        road.curveTo(54, 58, 40, 60, 46, 52);
        g.setStroke(new BasicStroke(13f, BasicStroke.CAP_BUTT, BasicStroke.JOIN_ROUND));
        g.setColor(new Color(0xD9B48A));
        g.draw(road);
        g.setStroke(new BasicStroke(1.6f, BasicStroke.CAP_ROUND, BasicStroke.JOIN_ROUND, 1f, new float[]{4f, 4f}, 0f));
        g.setColor(new Color(0xFFF6E8));
        g.draw(road);
        // car (side view)
        g.setColor(new Color(0x00000030, true));
        g.fill(new Ellipse2D.Double(22, 79, 46, 6));
        g.setColor(new Color(0xE8564F));
        g.fill(new RoundRectangle2D.Double(22, 66, 46, 13, 8, 8));
        Path2D cabin = new Path2D.Double();
        cabin.moveTo(32, 67); cabin.lineTo(38, 57); cabin.lineTo(53, 57); cabin.lineTo(60, 67); cabin.closePath();
        g.fill(cabin);
        g.setColor(new Color(0xCFEFFF));
        Path2D win = new Path2D.Double();
        win.moveTo(36, 66); win.lineTo(40, 59.5); win.lineTo(51.5, 59.5); win.lineTo(56, 66); win.closePath();
        g.fill(win);
        g.setColor(new Color(0xFFFFFF));
        g.fill(new RoundRectangle2D.Double(42, 69, 8, 6, 3, 3));
        g.setColor(new Color(0x3A3440));
        g.fill(new Ellipse2D.Double(26, 73, 11, 11));
        g.fill(new Ellipse2D.Double(53, 73, 11, 11));
        g.setColor(new Color(0xC8C2CC));
        g.fill(new Ellipse2D.Double(29, 76, 5, 5));
        g.fill(new Ellipse2D.Double(56, 76, 5, 5));
        g.dispose();
        return img;
    }
}
